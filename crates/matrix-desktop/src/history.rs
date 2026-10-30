use matrix_domain::{EventId, RoomScope, ScopedTimelineId, TimelineId, TransactionId};
use matrix_engine::history::HistorySender;

/// Offline presentation workload, not server history or canonical SDK state.
pub struct DemoHistory {
    events: Vec<DemoEvent>,
    scope: RoomScope,
    older_count: usize,
    local_count: usize,
    hide_redacted: bool,
    /// Redacted rows kept out of `events` while hidden.
    hidden: Vec<DemoEvent>,
}

/// Demo transport controls only; neither state means delivery to a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemoSendState {
    Pending,
    Failed,
}

/// What a video bubble draws besides the picture: ready-made duration and size labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFacts {
    pub duration: Option<String>,
    pub size: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DemoEvent {
    /// Observed server event/ACK, never a peer read receipt or fixture inference.
    pub server_confirmed: bool,
    pub redacted: bool,
    pub transaction_id: Option<TransactionId>,
    pub id: ScopedTimelineId,
    pub text: String,
    /// Bold, italic, code… of `text` for a formatted message.
    pub styles: Vec<matrix_engine::rich_text::StyleSpan>,
    pub annotation: String,
    pub outgoing: bool,
    pub time: String,
    pub send_state: Option<DemoSendState>,
    /// None for local/own rows.
    pub sender: Option<HistorySender>,
    pub timestamp_ms: u64,
    /// Set for an `m.video` row: the bubble paints a picture with play symbol instead of `text`.
    pub video: Option<VideoFacts>,
    /// File name of a received attachment (any kind): the bubble offers «Scarica».
    pub attachment_name: Option<String>,
    /// Set on a thread root: replies count and who wrote the latest one.
    pub thread: Option<matrix_engine::history::ThreadInfo>,
    /// Set on a reply inside a thread: the thread's root.
    pub thread_root: Option<matrix_domain::EventId>,
    /// A status row (join, invite, rename…): drawn as a quiet centred line, not a bubble.
    pub status: bool,
}

impl DemoEvent {
    /// Bubble footer text; never contains the event id.
    pub fn stamp(&self) -> String {
        bubble_stamp(&self.time, self.outgoing, self.send_state)
    }

    /// Footer for a 1:1 row: a read own message gets the double check.
    pub fn stamp_read(&self, read: bool) -> String {
        if read && self.outgoing && self.send_state.is_none() {
            format!("{} ✓✓", self.time)
        } else {
            self.stamp()
        }
    }

    /// «N risposte» under a thread root, with the initials for the latest author's avatar.
    pub fn thread_label(&self) -> Option<(String, String, Option<&str>)> {
        let t = self.thread.as_ref()?;
        let mut label = match t.num_replies {
            1 => "1 risposta".to_owned(),
            n => format!("{n} risposte"),
        };
        match t.unread {
            0 => {}
            1 => label.push_str(" · 1 non letta"),
            n => label.push_str(&format!(" · {n} non lette")),
        }
        let who = t.latest_sender.as_ref().map(|s| s.name.as_deref().unwrap_or(&s.id));
        let initials = who.unwrap_or("").trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
        Some((label, initials, t.latest_sender.as_ref().and_then(|s| s.avatar.as_deref())))
    }

    /// Server event id shown in debug mode; unsent rows have none.
    pub fn debug_id(&self) -> Option<&str> {
        match &self.id.id {
            TimelineId::Server(id) => Some(id.as_str()),
            _ => None,
        }
    }

    /// Whether the message offers its actions menu: confirmed server messages only.
    pub fn menu_available(&self, live: bool) -> bool {
        live && !self.status && self.send_state.is_none()
    }
}

/// Debug-mode line above the timeline: room id and sync state.
pub fn debug_room_line(scope: &matrix_domain::RoomScope, sync: &str) -> String {
    format!("{} · {sync}", scope.room_id.as_str())
}

/// Menu key or Shift+F10, the platform keys that open a context menu.
pub fn opens_message_menu(
    key: &str,
    shift: bool,
    control: bool,
    alt: bool,
    platform: bool,
) -> bool {
    !control && !alt && !platform && (key == "menu" || (key == "f10" && shift))
}

impl DemoHistory {
    /// The thread was read in its panel: its root shows no unread replies until the engine says otherwise.
    pub fn clear_thread_unread(&mut self, root: &EventId) -> bool {
        let row = self.events.iter_mut().find(|row| row.id.id == TimelineId::Server(root.clone()));
        row.and_then(|r| r.thread.as_mut()).is_some_and(|t| std::mem::take(&mut t.unread) > 0)
    }

    pub fn row(&self, id: &EventId) -> Option<&DemoEvent> {
        self.events.iter().find(|row| row.id.id == TimelineId::Server(id.clone()))
    }

    /// Message the keyboard menu opens on: `focused` when it still offers one, else the newest.
    pub fn keyboard_menu_target(
        &self,
        focused: Option<&ScopedTimelineId>,
    ) -> Option<ScopedTimelineId> {
        let offers =
            |e: &&DemoEvent| e.menu_available(true) && matches!(e.id.id, TimelineId::Server(_));
        self.events()
            .iter()
            .filter(offers)
            .find(|e| Some(&e.id) == focused)
            .or_else(|| self.events().iter().rev().find(offers))
            .map(|e| e.id.clone())
    }

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
                    styles: Vec::new(),
                    thread: None,
                    thread_root: None,
                    status: false,
                    video: None,
                    attachment_name: None,
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
            hide_redacted: false,
            hidden: Vec::new(),
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
            styles: Vec::new(),
            thread: None,
            thread_root: None,
            status: false,
            video: None,
            attachment_name: None,
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
                && (row.text != event.body
                    || row.styles != event.styles
                    || row.redacted != event.redacted)
            {
                row.redacted = event.redacted;
                row.text.clone_from(&event.body);
                row.styles.clone_from(&event.styles);
                changed_body = true;
            }
            // The count moves when a reply arrives, the text does not.
            if let Some(row) = self.events.iter_mut().find(|row| row.id.id == TimelineId::Server(event.event_id.clone()))
                && row.thread != event.thread
            {
                row.thread.clone_from(&event.thread);
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
        let mut seen: std::collections::HashSet<_> = self
            .events
            .iter()
            .chain(&self.hidden)
            .map(|row| row.id.id.clone())
            .collect();
        let mut rows = events
            .into_iter()
            .filter(|event| seen.insert(TimelineId::Server(event.event_id.clone())))
            .map(|event| DemoEvent {
                server_confirmed: true,
                redacted: event.redacted,
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
                styles: event.styles,
                thread: event.thread,
                thread_root: event.thread_root,
                status: event.status,
                attachment_name: event.attachment_name,
                video: event.video.map(|v| VideoFacts {
                    duration: v.duration_ms.map(matrix_engine::history::duration_label),
                    size: v.size.map(matrix_engine::history::human_size),
                }),
            })
            .collect::<Vec<_>>();
        let count = rows
            .iter()
            .filter(|r| !(self.hide_redacted && r.redacted))
            .count();
        if append {
            self.events.append(&mut rows);
        } else {
            rows.append(&mut self.events);
            self.events = rows;
        }
        self.sweep_redacted();
        count
    }

    /// «Nascondi messaggi eliminati»: shows or hides the redacted rows already loaded.
    /// Returns whether the visible rows changed.
    pub fn set_hide_redacted(&mut self, hide: bool) -> bool {
        if self.hide_redacted == hide {
            return false;
        }
        self.hide_redacted = hide;
        if hide {
            self.sweep_redacted();
        } else {
            // shortcut: order by origin_server_ts, not by SDK position; revisit if the timeline shows events out of time order.
            for row in std::mem::take(&mut self.hidden) {
                let at = self
                    .events
                    .iter()
                    .position(|e| e.timestamp_ms > row.timestamp_ms)
                    .unwrap_or(self.events.len());
                self.events.insert(at, row);
            }
        }
        true
    }

    /// While hiding, moves redacted rows out of `events`; neighbours then group as if they never existed.
    fn sweep_redacted(&mut self) {
        if !self.hide_redacted {
            return;
        }
        let mut kept: Vec<DemoEvent> = Vec::with_capacity(self.events.len());
        for row in std::mem::take(&mut self.events) {
            if row.redacted {
                self.hidden.push(row);
            } else {
                kept.push(row);
            }
        }
        self.events = kept;
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
                row.styles.clear();
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
                    styles: Vec::new(),
                    thread: None,
                    thread_root: None,
                    status: false,
                    video: None,
                    attachment_name: None,
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
                row.styles.clear();
                row.time = format!(
                    "{} · modificato",
                    row.time.trim_end_matches(" · modificato")
                );
            }
        }
        self.sweep_redacted();
    }

    /// Reconcile outgoing rows by transaction/event ID, never by message text.
    pub fn project_delivery(&mut self, items: &[matrix_domain::TimelineItem]) {
        // An unsent message the delivery no longer holds was withdrawn from the send queue.
        self.events.retain(|e| {
            e.send_state.is_none()
                || e.transaction_id.is_none()
                || items.iter().any(|i| i.transaction_id == e.transaction_id)
        });
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
                    row.styles.clear();
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
                    styles: Vec::new(),
                    thread: None,
                    thread_root: None,
                    status: false,
                    video: None,
                    attachment_name: None,
                });
            }
        }
    }

    /// Index of the newest event the peer has read; everything before it is read too.
    pub fn peer_read_index(
        &self,
        receipts: &[matrix_engine::receipts::PublicReadReceipt],
    ) -> Option<usize> {
        self.events.iter().rposition(|e| match &e.id.id {
            TimelineId::Server(id) => receipts.iter().any(|r| r.readers > 0 && r.event_id == *id),
            _ => false,
        })
    }

    pub fn events(&self) -> &[DemoEvent] {
        &self.events
    }

    /// Avatar beside bubble `index`, drawn where the sender name would be: the picture's mxc
    /// (None = no picture, show the initials) and the initials. Same rule as the name.
    pub fn sender_avatar(&self, index: usize, member_count: u64) -> Option<(Option<&str>, String)> {
        let name = self.sender_label(index, member_count)?;
        let sender = self.events.get(index)?.sender.as_ref()?;
        let initials = name.trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
        Some((sender.avatar.as_deref(), initials))
    }

    /// Pictures the senders of this room's bubbles have.
    pub fn sender_avatar_mxcs(&self) -> impl Iterator<Item = &str> {
        self.events.iter().rev().flat_map(|e| {
            let latest = e.thread.as_ref().and_then(|t| t.latest_sender.as_ref());
            e.sender.iter().chain(latest).filter_map(|s| s.avatar.as_deref())
        })
    }

    /// Sender name to draw above bubble `index`, per `matrix_domain::shows_sender_label`.
    /// Falls back to the Matrix ID when the sender has no display name.
    pub fn sender_label(&self, index: usize, member_count: u64) -> Option<String> {
        let event = self.events.get(index)?;
        let sender = event.sender.as_ref()?;
        let previous = index.checked_sub(1).map(|i| &self.events[i]).map(|prev| {
            let same = !prev.status && prev.sender.as_ref().is_some_and(|p| p.id == sender.id);
            (same, event.timestamp_ms.saturating_sub(prev.timestamp_ms))
        });
        matrix_domain::shows_sender_label(
            member_count.try_into().unwrap_or(u32::MAX),
            event.outgoing,
            previous,
        )
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

#[cfg(test)]
mod sender_avatar_tests {
    use super::*;

    #[test]
    fn avatar_beside_the_sender_name_with_initials_when_there_is_no_picture() {
        let scope = RoomScope {
            account_id: matrix_domain::AccountId::new(0),
            room_id: matrix_domain::RoomId::new("!r:fixture.invalid"),
        };
        let mut history = DemoHistory::new(scope, 4);
        let with = HistorySender::new("@bruno:fixture.invalid", None)
            .with_avatar(Some("mxc://fixture.invalid/b".into()));
        history.events[0].sender = Some(with);
        // Index 0 has a picture, index 2 (Anna) none; index 1 is our own message.
        assert_eq!(
            history.sender_avatar(0, 3),
            Some((Some("mxc://fixture.invalid/b"), "BR".to_owned()))
        );
        assert_eq!(history.sender_avatar(2, 3), Some((None, "AN".to_owned())));
        assert_eq!(history.sender_avatar(1, 3), None);
        // A chat of two: the header already shows the other person, so no name and no avatar.
        assert_eq!(history.sender_avatar(0, 2), None);
        assert_eq!(
            history.sender_avatar_mxcs().collect::<Vec<_>>(),
            ["mxc://fixture.invalid/b"]
        );
    }
}
