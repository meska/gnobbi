use matrix_engine::history::HistorySender;
use matrix_domain::{EventId, RoomScope, ScopedTimelineId, TimelineId, TransactionId};

/// Offline presentation workload, not server history or canonical SDK state.
pub struct DemoHistory {
    events: Vec<DemoEvent>,
    scope: RoomScope,
    older_count: usize,
    local_count: usize,
}

/// Demo transport controls only; neither state means delivery to a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemoSendState {
    Pending,
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DemoEvent {
    /// Observed server event/ACK, never a peer read receipt or fixture inference.
    pub server_confirmed: bool,
    pub redacted: bool,
    pub transaction_id: Option<TransactionId>,
    pub id: ScopedTimelineId,
    pub text: String,
    pub annotation: String,
    pub outgoing: bool,
    pub time: String,
    pub send_state: Option<DemoSendState>,
    /// None for local/own rows.
    pub sender: Option<HistorySender>,
    pub timestamp_ms: u64,
}

impl DemoHistory {
    pub fn new(scope: RoomScope, count: usize) -> Self {
        let events = (0..count)
            .map(|index| {
                let body = match index % 5 {
                    0 => "Una fixture offline, nessun messaggio di rete.".to_owned(),
                    1 => "Un caffè in città: testo Unicode originale. Il core rimane indipendente dalla UI. ".repeat(7),
                    2 => "Prima riga esplicita.\nSeconda riga: niente homeserver connesso.\nTerza riga completa.".to_owned(),
                    3 => "[Anteprima locale: media non caricati]\nSpazio riservato nella fixture, nessun download.".to_owned(),
                    _ => "Link demo: https://example.invalid/docs?x=1&y=2\nNon apribile: javascript:alert(1)".to_owned(),
                };
                DemoEvent {
                    server_confirmed: false,
                    redacted: false,
                    annotation: String::new(),
                    transaction_id: None,
                    id: ScopedTimelineId {
                        scope: scope.clone(),
                        id: TimelineId::Server(EventId::new(format!("$demo-{}:fixture.invalid", index + 1))),
                    },
                    text: format!("Evento {:05} · {body}", index + 1),
                    outgoing: index % 3 == 1,
                    send_state: None,
                    timestamp_ms: index as u64 * 60_000,
                    sender: (index % 3 != 1).then(|| {
                        let (id, name) = [("@bruno:fixture.invalid", None), ("@anna:fixture.invalid", Some("Anna"))][index / 2 % 2];
                        HistorySender::new(id, name)
                    }),
                    time: format!("{:02}:{:02}", index / 60 % 24, index % 60),
                }
            })
            .collect();
        Self {
            events,
            scope,
            older_count: 0,
            local_count: 0,
        }
    }

    /// Insert deterministic older offline events; existing identities/content do not change.
    pub fn prepend_older(&mut self, count: usize) {
        let end = self
            .older_count
            .checked_add(count)
            .expect("demo count overflow");
        let mut older = DemoHistory::new(self.scope.clone(), count).events;
        for (event, sequence) in older.iter_mut().zip((self.older_count + 1..=end).rev()) {
            event.id.id =
                TimelineId::Server(EventId::new(format!("$older-{sequence}:fixture.invalid")));
            event.text = format!("Storico {sequence:05} · {}", event.text);
        }
        older.append(&mut self.events);
        self.events = older;
        self.older_count = end;
    }

    /// Append a clearly labelled simulated arrival; never sends or receives network data.
    pub fn append_live(&mut self, count: usize) {
        let start = self.events.len() - self.older_count;
        let end = start.checked_add(count).expect("demo count overflow");
        let mut live = DemoHistory::new(self.scope.clone(), count).events;
        for (event, sequence) in live.iter_mut().zip(start + 1..=end) {
            event.id.id =
                TimelineId::Server(EventId::new(format!("$demo-{sequence}:fixture.invalid")));
            event.text = format!("Ricezione simulata {sequence:05} · {}", event.text);
        }
        self.events.append(&mut live);
    }

    /// Local composer simulation only: no server ACK or delivery claim.
    pub fn submit_text(&mut self, text: String) {
        self.local_count = self
            .local_count
            .checked_add(1)
            .expect("demo count overflow");
        self.events.push(DemoEvent {
            server_confirmed: false,
            redacted: false,
            annotation: String::new(),
            transaction_id: None,
            id: ScopedTimelineId {
                scope: self.scope.clone(),
                id: TimelineId::Pending(TransactionId::new(format!(
                    "demo-local-{}",
                    self.local_count
                ))),
            },
            text,
            outgoing: true,
            time: "locale".into(),
            send_state: Some(DemoSendState::Pending),
            sender: None,
            timestamp_ms: 0,
        });
    }

    /// Explicit fixture control, not a transport error or automatic network timeout.
    pub fn fail_local(&mut self, id: &ScopedTimelineId) -> bool {
        if let Some(event) = self.events.iter_mut().find(|event| &event.id == id)
            && event.send_state == Some(DemoSendState::Pending)
        {
            event.send_state = Some(DemoSendState::Failed);
            return true;
        }
        false
    }

    /// Retry the same local transaction; never creates a second message or server ACK.
    pub fn retry_local(&mut self, id: &ScopedTimelineId) -> bool {
        if let Some(event) = self.events.iter_mut().find(|event| &event.id == id)
            && event.send_state == Some(DemoSendState::Failed)
        {
            event.send_state = Some(DemoSendState::Pending);
            return true;
        }
        false
    }

    /// Render-only row conversion after HistorySession validates/deduplicates a page.
    /// Live consumers initialize with count0: no fixture or simulated send is added.
    pub fn prepend_server(&mut self, events: Vec<matrix_engine::history::HistoryEvent>) -> usize {
        self.insert_server(events, false)
    }

    pub fn append_server(&mut self, events: Vec<matrix_engine::history::HistoryEvent>) -> usize {
        self.insert_server(events, true)
    }

    /// Current scoped snapshots may replace loaded bodies; older pages/ACKs may not.
    pub fn refresh_server(&mut self, events: Vec<matrix_engine::history::HistoryEvent>) -> bool {
        let mut changed_body = false;
        for event in &events {
            if let Some(row) = self
                .events
                .iter_mut()
                .find(|row| row.id.id == TimelineId::Server(event.event_id.clone()))
                && row.text != event.body
            {
                row.text.clone_from(&event.body);
                changed_body = true;
            }
        }
        self.append_server(events);
        changed_body
    }

    fn insert_server(
        &mut self,
        events: Vec<matrix_engine::history::HistoryEvent>,
        append: bool,
    ) -> usize {
        let mut seen: std::collections::HashSet<_> =
            self.events.iter().map(|row| row.id.id.clone()).collect();
        let mut rows = events
            .into_iter()
            .filter(|event| seen.insert(TimelineId::Server(event.event_id.clone())))
            .map(|event| DemoEvent {
                server_confirmed: true,
                redacted: false,
                annotation: String::new(),
                transaction_id: None,
                id: ScopedTimelineId {
                    scope: self.scope.clone(),
                    id: TimelineId::Server(event.event_id),
                },
                text: event.body,
                outgoing: event.outgoing,
                time: format!(
                    "{:02}:{:02} UTC",
                    event.timestamp_ms / 3_600_000 % 24,
                    event.timestamp_ms / 60_000 % 60
                ),
                send_state: None,
                sender: (!event.outgoing).then_some(event.sender),
                timestamp_ms: event.timestamp_ms,
            })
            .collect::<Vec<_>>();
        let count = rows.len();
        if append {
            self.events.append(&mut rows);
        } else {
            rows.append(&mut self.events);
            self.events = rows;
        }
        count
    }

    pub fn project_operations(
        &mut self,
        operations: &[crate::live_operations::ConfirmedOperation],
    ) {
        for row in &mut self.events {
            row.annotation.clear();
        }
        for operation in operations
            .iter()
            .filter(|op| op.request.scope == self.scope)
        {
            if matches!(
                operation.request.operation,
                matrix_engine::operations::Operation::Redact
            ) && let Some(row) = self
                .events
                .iter_mut()
                .find(|row| row.id.id == TimelineId::Server(operation.request.target.clone()))
            {
                row.text = "[Messaggio eliminato]".into();
                row.redacted = true;
                row.annotation.clear();
            }
            if let matrix_engine::operations::Operation::React { key } =
                &operation.request.operation
                && !operations.iter().any(|redact| {
                    redact.request.scope == self.scope
                        && redact.request.target == operation.event_id
                        && redact.request.operation == matrix_engine::operations::Operation::Redact
                })
                && let Some(row) = self
                    .events
                    .iter_mut()
                    .find(|row| row.id.id == TimelineId::Server(operation.request.target.clone()))
            {
                if !row.annotation.is_empty() {
                    row.annotation.push_str(" · ");
                }
                row.annotation.push_str(&format!("{key} · tu"));
            }
            if let matrix_engine::operations::Operation::Reply { body } =
                &operation.request.operation
                && !self
                    .events
                    .iter()
                    .any(|row| row.id.id == TimelineId::Server(operation.event_id.clone()))
            {
                self.events.push(DemoEvent {
                    annotation: String::new(),
                    transaction_id: None,
                    server_confirmed: true,
                    redacted: false,
                    id: ScopedTimelineId {
                        scope: self.scope.clone(),
                        id: TimelineId::Server(operation.event_id.clone()),
                    },
                    text: body.clone(),
                    outgoing: true,
                    time: "ACK server · Risposta".into(),
                    send_state: None,
                    sender: None,
                    timestamp_ms: 0,
                });
            }
            if let matrix_engine::operations::Operation::Edit { body } =
                &operation.request.operation
                && let Some(row) = self
                    .events
                    .iter_mut()
                    .find(|row| row.id.id == TimelineId::Server(operation.request.target.clone()))
            {
                row.text = body.clone();
                row.time = format!(
                    "{} · modificato",
                    row.time.trim_end_matches(" · modificato")
                );
            }
        }
    }

    /// Reconcile outgoing rows by transaction/event ID, never by message text.
    pub fn project_delivery(&mut self, items: &[matrix_domain::TimelineItem]) {
        for item in items.iter().filter(|i| i.identity.scope == self.scope) {
            let Some(tx) = &item.transaction_id else {
                continue;
            };
            let local = self
                .events
                .iter()
                .position(|e| e.transaction_id.as_ref() == Some(tx));
            let remote = self.events.iter().position(|e| e.id == item.identity);
            let index = remote.or(local);
            let state = match item.status {
                matrix_domain::SendStatus::Pending => Some(DemoSendState::Pending),
                matrix_domain::SendStatus::Failed => Some(DemoSendState::Failed),
                matrix_domain::SendStatus::Sent => None,
            };
            if let Some(index) = index {
                let row = &mut self.events[index];
                row.id = item.identity.clone();
                row.transaction_id = Some(tx.clone());
                // Delivery ACKs carry the original body, not authoritative edited/redacted content.
                if !row.server_confirmed {
                    row.text = item.body.clone();
                }
                row.send_state = state;
                row.server_confirmed = state.is_none();
                if state.is_none() {
                    row.time = "ACK server".into();
                }
                if let Some(local) = local.filter(|r| *r != index) {
                    self.events.remove(local);
                }
            } else {
                self.events.push(DemoEvent {
                    annotation: String::new(),
                    id: item.identity.clone(),
                    server_confirmed: state.is_none(),
                    redacted: false,
                    transaction_id: Some(tx.clone()),
                    text: item.body.clone(),
                    outgoing: true,
                    time: if state.is_some() {
                        "locale".into()
                    } else {
                        "ACK server".into()
                    },
                    send_state: state,
                    sender: None,
                    timestamp_ms: 0,
                });
            }
        }
    }

    pub fn events(&self) -> &[DemoEvent] {
        &self.events
    }

    /// Sender name to draw above bubble `index`, per `matrix_domain::shows_sender_label`.
    /// Falls back to the Matrix ID when the sender has no display name.
    pub fn sender_label(&self, index: usize, member_count: u64) -> Option<String> {
        let event = self.events.get(index)?;
        let sender = event.sender.as_ref()?;
        let previous = index.checked_sub(1).map(|i| &self.events[i]).map(|prev| {
            let same = prev.sender.as_ref().is_some_and(|p| p.id == sender.id);
            (same, event.timestamp_ms.saturating_sub(prev.timestamp_ms))
        });
        matrix_domain::shows_sender_label(member_count.try_into().unwrap_or(u32::MAX), event.outgoing, previous)
            .then(|| sender.name.clone().unwrap_or_else(|| sender.id.clone()))
    }
}

/// Bubble footer: the time plus one glyph for the delivery state (no state words).
pub fn bubble_stamp(time: &str, outgoing: bool, state: Option<DemoSendState>) -> String {
    match (outgoing, state) {
        (_, Some(DemoSendState::Pending)) => format!("{time} ◷"),
        (_, Some(DemoSendState::Failed)) => format!("{time} !"),
        (true, None) => format!("{time} ✓"),
        (false, None) => time.to_owned(),
    }
}
