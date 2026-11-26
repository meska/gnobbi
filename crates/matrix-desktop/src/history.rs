use matrix_domain::{EventId, RoomScope, ScopedTimelineId, TimelineId, TransactionId};
use matrix_engine::history::HistorySender;

/// Offline presentation workload, not server history or canonical SDK state.
pub struct DemoHistory {
    events: Vec<DemoEvent>,
    scope: RoomScope,
    older_count: usize,
    local_count: usize,
    hide_redacted: bool,
    hide_status: bool,
    /// Redacted/status rows kept out of `events` while hidden.
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

fn video_facts(v: matrix_engine::history::VideoInfo) -> VideoFacts {
    VideoFacts {
        duration: v.duration_ms.map(matrix_engine::history::duration_label),
        size: v.size.map(matrix_engine::history::human_size),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DemoEvent {
    /// Observed server event/ACK, never a peer read receipt or fixture inference.
    pub server_confirmed: bool,
    pub redacted: bool,
    pub transaction_id: Option<TransactionId>,
    pub id: ScopedTimelineId,
    pub text: String,
    /// Bold, italic, code… of `text` for a formatted message.
    pub styles: Vec<matrix_engine::rich_text::StyleSpan>,
    /// Mention pills of `text` (users, «@room», rooms).
    pub mentions: Vec<matrix_engine::rich_text::MentionSpan>,
    /// The message is for the active user: the row is drawn highlighted.
    pub highlighted: bool,
    pub annotation: String,
    pub outgoing: bool,
    pub time: String,
    pub send_state: Option<DemoSendState>,
    /// None for local/own rows.
    pub sender: Option<HistorySender>,
    pub timestamp_ms: u64,
    /// Set for an `m.video` row: the bubble paints a picture with play symbol instead of `text`.
    pub video: Option<VideoFacts>,
    /// Set for an `m.audio` row: the bubble paints its own player (waveform, Play/Pause).
    pub audio: Option<matrix_engine::history::AudioInfo>,
    /// Set on an image, video, audio or file row: `text` (with `styles`, `mentions`) is then its caption, empty when none.
    pub media: Option<matrix_engine::history::Media>,
    /// Set on a poll start: the bubble paints the poll instead of `text`. Private user data: never log.
    pub poll: Option<matrix_domain::Poll>,
    /// File name of a received attachment (any kind): the bubble offers «Scarica».
    pub attachment_name: Option<String>,
    pub attachment_size: Option<u64>,
    /// Set on a thread root: replies count and who wrote the latest one.
    pub thread: Option<matrix_engine::history::ThreadInfo>,
    /// Set on a reply inside a thread: the thread's root.
    pub thread_root: Option<matrix_domain::EventId>,
    /// Set on a reply: the quote drawn at the top of the bubble.
    pub reply_to: Option<matrix_engine::history::ReplyRef>,
    /// A status row (join, invite, rename…): drawn as a quiet centred line, not a bubble.
    pub status: bool,
    /// Set on a status row: what it says; `text` is empty, `status_text` words it in the active language.
    pub status_row: Option<matrix_engine::status_line::StatusRow>,
    /// Could not be decrypted: the row offers «Recupera chiavi» while the recovery is incomplete.
    pub utd: bool,
    /// Reactions under the message (server state; the pending ones of this session are laid over it at draw time).
    pub reactions: Vec<matrix_engine::history::Reaction>,
}

impl DemoEvent {
    /// Bubble footer text; never contains the event id.
    pub fn stamp(&self) -> String {
        bubble_stamp(&self.time, self.outgoing, self.send_state)
    }

    /// The footer as time and delivery mark, for a UI that draws the mark as an icon.
    pub fn stamp_parts(&self) -> (&str, Option<StampMark>) {
        (&self.time, stamp_mark(self.outgoing, self.send_state))
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
        let mut label = crate::i18n::plural(t.num_replies as u64, "{n} reply", "{n} replies");
        if t.unread > 0 {
            label.push_str(" · ");
            label.push_str(&crate::i18n::plural(t.unread as u64, "{n} unread", "{n} unread"));
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

    pub fn scope(&self) -> &RoomScope {
        &self.scope
    }

    /// A history holding one row already known (a thread's root, taken from the room's timeline). The row goes in as it
    /// is, not through `insert_server`: it is already a rendered row, and its id and thread summary must stay untouched.
    pub fn seeded(scope: RoomScope, row: DemoEvent) -> Self {
        let mut history = Self::new(scope, 0);
        history.events.push(row);
        history
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
                    annotation: String::new(), reactions: Vec::new(),
                    transaction_id: None,
                    id: ScopedTimelineId {
                        scope: scope.clone(),
                        id: TimelineId::Server(EventId::new(format!("$demo-{}:fixture.invalid", index + 1))),
                    },
                    text: format!("Evento {:05} · {body}", index + 1),
                    outgoing: index % 3 == 1,
                    send_state: None,
                    timestamp_ms: index as u64 * 60_000,
                    styles: Vec::new(), mentions: Vec::new(), highlighted: false,
                    thread: None,
                    thread_root: None,
                    reply_to: None,
                    status: false, status_row: None,
                    utd: false,
                    video: None,
                    audio: None,
                    poll: None,
                    attachment_name: None,
                    attachment_size: None, media: None,
                    sender: (index % 3 != 1).then(|| {
                        let (id, name) = [("@bruno:fixture.invalid", None), ("@anna:fixture.invalid", Some("Anna"))][index / 2 % 2];
                        HistorySender::new(id, name)
                    }),
                    time: crate::time_fmt::clock_time(index as u64 * 60_000),
                }
            })
            .collect();
        Self {
            events,
            scope,
            older_count: 0,
            local_count: 0,
            hide_redacted: false,
            hide_status: false,
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
            annotation: String::new(), reactions: Vec::new(),
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
            time: crate::i18n::t("local").into(),
            send_state: Some(DemoSendState::Pending),
            sender: None,
            timestamp_ms: 0,
            styles: Vec::new(), mentions: Vec::new(), highlighted: false,
            thread: None,
            thread_root: None,
            reply_to: None,
            status: false, status_row: None,
            utd: false,
            video: None,
            audio: None,
            poll: None,
            attachment_name: None,
            attachment_size: None, media: None,
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
            // Own echo/ACK rows start at ts 0; take the server ts before any redaction hides them.
            if let Some(row) = self
                .events
                .iter_mut()
                .find(|row| row.id.id == TimelineId::Server(event.event_id.clone()))
                && row.timestamp_ms == 0
            {
                row.timestamp_ms = event.timestamp_ms;
                if event.timestamp_ms != 0 {
                    let edited = row.time.ends_with(&format!(" · {}", crate::i18n::t("edited")));
                    row.time = crate::time_fmt::clock_time(event.timestamp_ms);
                    if edited {
                        row.time.push_str(&format!(" · {}", crate::i18n::t("edited")));
                    }
                }
            }
            if let Some(row) = self
                .events
                .iter_mut()
                .find(|row| row.id.id == TimelineId::Server(event.event_id.clone()))
                && (row.text != event.body
                    || row.utd != event.utd
                    || row.styles != event.styles
                    || row.mentions != event.mentions
                    || row.highlighted != event.highlighted
                    || row.poll != event.poll
                    || row.media != event.media
                    || row.redacted != event.redacted
                    || row.reactions != event.reactions)
            {
                row.reactions.clone_from(&event.reactions);
                row.media.clone_from(&event.media);
                row.poll.clone_from(&event.poll);
                row.redacted = event.redacted;
                row.utd = event.utd;
                row.text.clone_from(&event.body);
                row.styles.clone_from(&event.styles);
                row.mentions.clone_from(&event.mentions);
                row.highlighted = event.highlighted;
                changed_body = true;
            }
            // The ACK row is born without attachment facts and the server copy is then dropped as a duplicate.
            if let Some(row) = self.events.iter_mut().find(|row| row.id.id == TimelineId::Server(event.event_id.clone()))
                && row.attachment_name.is_none()
                && event.attachment_name.is_some()
            {
                row.attachment_name.clone_from(&event.attachment_name);
                row.attachment_size = event.attachment_size;
                row.video = event.video.clone().map(video_facts);
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
        // A delivery echo row is born without the poll; the history copy would be dropped as duplicate.
        for event in events.iter().filter(|e| e.poll.is_some()) {
            if let Some(row) = self.events.iter_mut().find(|r| r.id.id == TimelineId::Server(event.event_id.clone()))
                && row.poll != event.poll
            {
                row.poll.clone_from(&event.poll);
                row.text.clone_from(&event.body);
            }
        }
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
                reactions: event.reactions,
                transaction_id: None,
                id: ScopedTimelineId {
                    scope: self.scope.clone(),
                    id: TimelineId::Server(event.event_id),
                },
                text: crate::row_text::engine_placeholder(&event.body).to_owned(),
                outgoing: event.outgoing,
                time: crate::time_fmt::clock_time(event.timestamp_ms),
                send_state: None,
                sender: (!event.outgoing).then_some(event.sender),
                timestamp_ms: event.timestamp_ms,
                styles: event.styles,
                mentions: event.mentions,
                highlighted: event.highlighted,
                thread: event.thread,
                thread_root: event.thread_root,
                reply_to: event.reply_to,
                status: event.status,
                status_row: event.status_row,
                utd: event.utd,
                attachment_name: event.attachment_name,
                attachment_size: event.attachment_size,
                media: event.media,
                video: event.video.map(video_facts),
                audio: event.audio,
                poll: event.poll,
            })
            .collect::<Vec<_>>();
        let count = rows
            .iter()
            .filter(|r| !self.hides(r))
            .count();
        if append {
            self.events.append(&mut rows);
        } else {
            rows.append(&mut self.events);
            self.events = rows;
        }
        self.sweep_hidden();
        count
    }

    fn hides(&self, row: &DemoEvent) -> bool {
        (self.hide_redacted && row.redacted && row.thread.is_none()) || (self.hide_status && row.status && !row.highlighted)
    }

    /// «Nascondi messaggi eliminati»: shows or hides the redacted rows already loaded.
    /// Returns whether the visible rows changed.
    pub fn set_hide_redacted(&mut self, hide: bool) -> bool {
        if self.hide_redacted == hide {
            return false;
        }
        self.hide_redacted = hide;
        self.reproject();
        true
    }

    /// Status rows kept out of the timeline right now, for the room panel.
    pub fn hidden_status_count(&self) -> usize {
        self.hidden.iter().filter(|r| r.status).count()
    }

    /// «Messaggi di stato» (global setting resolved with the room's choice); a status row marked `highlighted` (an invitation for you) always stays.
    pub fn set_hide_status(&mut self, hide: bool) -> bool {
        if self.hide_status == hide {
            return false;
        }
        self.hide_status = hide;
        self.reproject();
        true
    }

    fn reproject(&mut self) {
        self.sweep_hidden();
        // shortcut: order by origin_server_ts, not by SDK position; revisit if the timeline shows events out of time order.
        let (still, back): (Vec<_>, Vec<_>) = std::mem::take(&mut self.hidden)
            .into_iter()
            .partition(|r| self.hides(r));
        self.hidden = still;
        for row in back {
            // ts 0 = row not yet synced: it goes to the tail.
            let at = self
                .events
                .iter()
                .position(|e| row.timestamp_ms != 0 && (e.timestamp_ms == 0 || e.timestamp_ms > row.timestamp_ms))
                .unwrap_or(self.events.len());
            self.events.insert(at, row);
        }
    }

    /// Moves hidden rows out of `events`; neighbours then group as if they never existed.
    fn sweep_hidden(&mut self) {
        let mut kept: Vec<DemoEvent> = Vec::with_capacity(self.events.len());
        for row in std::mem::take(&mut self.events) {
            if self.hides(&row) {
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
                row.text = crate::i18n::t("[Message deleted]").into();
                row.styles.clear();
                row.mentions.clear();
                row.redacted = true;
                row.annotation.clear();
            }
            if let matrix_engine::operations::Operation::Reply { body } =
                &operation.request.operation
                && !self
                    .events
                    .iter()
                    .any(|row| row.id.id == TimelineId::Server(operation.event_id.clone()))
            {
                self.events.push(DemoEvent {
                    annotation: String::new(), reactions: Vec::new(),
                    transaction_id: None,
                    server_confirmed: true,
                    redacted: false,
                    id: ScopedTimelineId {
                        scope: self.scope.clone(),
                        id: TimelineId::Server(operation.event_id.clone()),
                    },
                    text: body.clone(),
                    outgoing: true,
                    time: crate::i18n::t("Server ack · Reply").into(),
                    send_state: None,
                    sender: None,
                    timestamp_ms: 0,
                    styles: Vec::new(), mentions: Vec::new(), highlighted: false,
                    thread: None,
                    thread_root: None,
                    reply_to: None,
                    status: false, status_row: None,
                    utd: false,
                    video: None,
                    audio: None,
                    poll: None,
                    attachment_name: None,
                    attachment_size: None, media: None,
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
                row.mentions.clear();
                let edited = format!(" · {}", crate::i18n::t("edited"));
                row.time = format!("{}{edited}", row.time.trim_end_matches(edited.as_str()));
            }
        }
        self.sweep_hidden();
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
                    row.mentions.clear();
                }
                row.send_state = state;
                row.server_confirmed = state.is_none();
                // Once the server timestamp is known the row already shows the local clock time.
                if state.is_none() && row.timestamp_ms == 0 {
                    row.time = crate::i18n::t("Server ack").into();
                }
                if let Some(local) = local.filter(|r| *r != index) {
                    self.events.remove(local);
                }
            } else {
                self.events.push(DemoEvent {
                    annotation: String::new(), reactions: Vec::new(),
                    id: item.identity.clone(),
                    server_confirmed: state.is_none(),
                    redacted: false,
                    transaction_id: Some(tx.clone()),
                    text: item.body.clone(),
                    outgoing: true,
                    time: if state.is_some() {
                        crate::i18n::t("local").into()
                    } else {
                        crate::i18n::t("Server ack").into()
                    },
                    send_state: state,
                    sender: None,
                    timestamp_ms: 0,
                    styles: Vec::new(), mentions: Vec::new(), highlighted: false,
                    thread: None,
                    thread_root: None,
                    reply_to: None,
                    status: false, status_row: None,
                    utd: false,
                    video: None,
                    audio: None,
                    poll: None,
                    attachment_name: None,
                    attachment_size: None, media: None,
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

    /// Own message row -> who has read up to it as their newest read message (Element style: a reader's circle sits only on the last own message they read).
    pub fn readers_by_row(
        &self,
        receipts: &[matrix_engine::receipts::PublicReadReceipt],
    ) -> std::collections::HashMap<usize, Vec<matrix_engine::history::HistorySender>> {
        // One entry per user id: the same user can sit on several events (public + private, thread + main, stale marker), keep the newest row.
        let mut last = std::collections::HashMap::<&str, (usize, &matrix_engine::history::HistorySender)>::new();
        for r in receipts {
            let Some(at) = self.events.iter().position(|e| matches!(&e.id.id, TimelineId::Server(id) if *id == r.event_id)) else { continue };
            let Some(row) = self.events[..=at].iter().rposition(|e| e.outgoing && matches!(e.id.id, TimelineId::Server(_))) else { continue };
            for u in &r.users {
                let e = last.entry(u.id.as_str()).or_insert((row, u));
                if row > e.0 { *e = (row, u); }
            }
        }
        let mut rows = std::collections::HashMap::<usize, Vec<_>>::new();
        for (row, u) in last.into_values() { rows.entry(row).or_default().push(u.clone()); }
        for v in rows.values_mut() { v.sort_by(|a, b| a.id.cmp(&b.id)); }
        rows
    }

    /// Who read the own message `id`: every peer whose marker sits on it or on a later row, oldest read first, once each.
    pub fn readers_of(&self, id: &ScopedTimelineId, receipts: &[matrix_engine::receipts::PublicReadReceipt]) -> Vec<matrix_engine::history::HistorySender> {
        let Some(row) = self.events.iter().position(|e| e.id == *id && e.outgoing && matches!(e.id.id, TimelineId::Server(_))) else { return Vec::new() };
        let mut out = Vec::<matrix_engine::history::HistorySender>::new();
        for r in receipts {
            if !self.events[row..].iter().any(|e| matches!(&e.id.id, TimelineId::Server(id) if *id == r.event_id)) { continue }
            for u in &r.users {
                if !out.iter().any(|o| o.id == u.id) { out.push(u.clone()); }
            }
        }
        out
    }

    pub fn events(&self) -> &[DemoEvent] {
        &self.events
    }

    /// Day label to draw above row `index` when it is the first of its calendar day (system zone and locale).
    pub fn day_separator(&self, index: usize) -> Option<String> {
        self.day_separator_in(index, &chrono::Local, crate::time_fmt::active(), now_ms())
    }

    pub fn day_separator_in<Tz: chrono::TimeZone>(&self, index: usize, tz: &Tz, p: &crate::time_fmt::Patterns, now_ms: u64) -> Option<String> {
        let ms = self.events.get(index)?.timestamp_ms;
        let prev = index.checked_sub(1).map(|i| self.events[i].timestamp_ms);
        crate::time_fmt::starts_day_in(prev, ms, tz).then(|| crate::time_fmt::day_label_in(ms, now_ms, tz, p))
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
            let pills = e.mentions.iter().filter_map(|m| match &m.kind {
                matrix_engine::rich_text::MentionKind::User { sender, .. } => Some(sender),
                _ => None,
            });
            e.sender.iter().chain(latest).chain(pills).filter_map(|s| s.avatar.as_deref())
        })
    }

    /// Sender name to draw above bubble `index`, per `matrix_domain::shows_sender_label`.
    /// Falls back to the Matrix ID when the sender has no display name.
    pub fn sender_label(&self, index: usize, member_count: u64) -> Option<String> {
        let event = self.events.get(index)?;
        let sender = event.sender.as_ref()?;
        let previous = index.checked_sub(1).map(|i| &self.events[i]).map(|prev| {
            (same_sender(prev, event), event.timestamp_ms.saturating_sub(prev.timestamp_ms))
        });
        matrix_domain::shows_sender_label(
            member_count.try_into().unwrap_or(u32::MAX),
            event.outgoing,
            previous,
        )
        .then(|| sender.name.clone().unwrap_or_else(|| sender.id.clone()))
    }
}

/// Previous bubble is a message from the same sender (status rows break a series).
fn same_sender(prev: &DemoEvent, event: &DemoEvent) -> bool {
    !prev.status
        && (prev.sender.as_ref().is_some_and(|p| event.sender.as_ref().is_some_and(|s| p.id == s.id))
            || (prev.outgoing && event.outgoing))
}

impl DemoHistory {
    /// Whether bubble `index` continues the series of the one above (same sender, same
    /// time window as the repeated sender name): the timeline then uses the tight gap.
    pub fn continues_series(&self, index: usize) -> bool {
        let Some(event) = self.events.get(index) else { return false };
        let Some(prev) = index.checked_sub(1).map(|i| &self.events[i]) else { return false };
        !event.status
            && same_sender(prev, event)
            && event.timestamp_ms.saturating_sub(prev.timestamp_ms) <= matrix_domain::SENDER_LABEL_GAP_MS
    }
}

fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// Delivery state shown next to the time of a bubble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampMark {
    Pending,
    Failed,
    Sent,
}

pub fn stamp_mark(outgoing: bool, state: Option<DemoSendState>) -> Option<StampMark> {
    match (outgoing, state) {
        (_, Some(DemoSendState::Pending)) => Some(StampMark::Pending),
        (_, Some(DemoSendState::Failed)) => Some(StampMark::Failed),
        (true, None) => Some(StampMark::Sent),
        (false, None) => None,
    }
}

/// Bubble footer as plain text (tests, spoken text): the time plus one glyph for the delivery state (no state words).
pub fn bubble_stamp(time: &str, outgoing: bool, state: Option<DemoSendState>) -> String {
    match stamp_mark(outgoing, state) {
        Some(StampMark::Pending) => format!("{time} ◷"),
        Some(StampMark::Failed) => format!("{time} !"),
        Some(StampMark::Sent) => format!("{time} ✓"),
        None => time.to_owned(),
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

#[cfg(test)]
mod day_separator_tests {
    use super::*;
    use chrono::FixedOffset;

    const HOUR: u64 = 3_600_000;
    // 2026-11-06 00:00 UTC.
    const DAY: u64 = 1_793_923_200_000;

    fn history(times: &[u64]) -> DemoHistory {
        let scope = RoomScope { account_id: matrix_domain::AccountId::new(0), room_id: matrix_domain::RoomId::new("!r:fixture.invalid") };
        let mut h = DemoHistory::new(scope, times.len());
        for (e, t) in h.events.iter_mut().zip(times) {
            e.timestamp_ms = *t;
        }
        h
    }

    fn marks(h: &DemoHistory, tz: &FixedOffset, now: u64) -> Vec<Option<String>> {
        let p = crate::time_fmt::Patterns::fallback();
        (0..h.events.len()).map(|i| h.day_separator_in(i, tz, &p, now)).collect()
    }

    #[test]
    fn label_only_where_the_day_changes_with_today_yesterday_and_older_names() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let h = history(&[DAY - 30 * 3 * 24 * HOUR, DAY + HOUR, DAY + 2 * HOUR, DAY + 25 * HOUR]);
        let m = marks(&h, &utc, DAY + 26 * HOUR);
        assert!(m[0].is_some(), "first row opens its day");
        assert_eq!(&m[1..], [Some("Yesterday".to_owned()), None, Some("Today".to_owned())]);
    }

    #[test]
    fn midnight_belongs_to_the_new_day_and_the_zone_decides_the_day() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let h = history(&[DAY - 1, DAY]);
        assert_eq!(marks(&h, &utc, DAY)[1].as_deref(), Some("Today"));
        // UTC+2: 23:59:59.999 UTC is 01:59 and midnight UTC is 02:00 — the same local day, no separator.
        let rome = FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(marks(&h, &rome, DAY)[1], None);
        // UTC-5 (UTC midnight is 19:00 the day before): the break falls at 05:00 UTC.
        let west = FixedOffset::west_opt(5 * 3600).unwrap();
        let h = history(&[DAY + 4 * HOUR, DAY + 5 * HOUR]);
        assert_eq!(marks(&h, &west, DAY + 6 * HOUR)[1].as_deref(), Some("Today"));
        assert_eq!(marks(&h, &utc, DAY + 6 * HOUR)[1], None);
    }
}

#[cfg(test)]
mod reader_circle_tests {
    use super::*;
    use matrix_engine::receipts::PublicReadReceipt;

    #[test]
    fn each_reader_lands_on_the_last_own_message_they_read() {
        let scope = RoomScope { account_id: matrix_domain::AccountId::new(0), room_id: matrix_domain::RoomId::new("!r:fixture.invalid") };
        let mut h = DemoHistory::new(scope, 4);
        for (i, id, own) in [(0, "$a", true), (1, "$b", false), (2, "$c", true)] {
            h.events[i].id.id = TimelineId::Server(matrix_domain::EventId::new(id));
            h.events[i].outgoing = own;
        }
        let at = |id: &str, who: &[&str]| PublicReadReceipt {
            event_id: matrix_domain::EventId::new(id),
            readers: who.len() as u64,
            users: who.iter().map(|w| HistorySender::new(w, None)).collect(),
        };
        // $b is foreign: its reader maps back to own $a; unknown ids are ignored.
        let rows = h.readers_by_row(&[at("$a", &["@x:t"]), at("$b", &["@y:t"]), at("$c", &["@z:t"]), at("$zz", &["@w:t"])]);
        assert_eq!(rows.get(&0).map(Vec::len), Some(2));
        assert_eq!(rows.get(&2).map(Vec::len), Some(1));
        assert_eq!(rows.len(), 2);

        // Same user on two events (and twice on one): one circle, on the newest own row.
        let rows = h.readers_by_row(&[at("$a", &["@x:t", "@y:t", "@y:t"]), at("$c", &["@x:t"])]);
        assert_eq!(rows.get(&2).map(|v| v.iter().map(|s| s.id.as_str()).collect::<Vec<_>>()), Some(vec!["@x:t"]));
        assert_eq!(rows.get(&0).map(|v| v.iter().map(|s| s.id.as_str()).collect::<Vec<_>>()), Some(vec!["@y:t"]));

        // The message menu: everyone at or past the own message, once; nothing for a foreign message or one nobody reached.
        let receipts = [at("$a", &["@x:t"]), at("$b", &["@y:t", "@x:t"]), at("$c", &["@z:t"])];
        let ids = |i: usize| h.readers_of(&h.events[i].id, &receipts).iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(0), ["@x:t", "@y:t", "@z:t"]);
        assert_eq!(ids(1), [""; 0], "someone else's message");
        assert_eq!(ids(2), ["@z:t"]);
        assert!(h.readers_of(&h.events[0].id, &[at("$zz", &["@w:t"])]).is_empty());
    }
}
