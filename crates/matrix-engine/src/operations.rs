//! Scoped message operations. Payloads intentionally do not implement Debug.
use matrix_domain::{EventId, RoomScope, TransactionId};

#[derive(Clone, PartialEq, Eq)]
pub enum Operation {
    Reply { body: String },
    Edit { body: String },
    React { key: String },
    Redact,
    /// Replace own votes with these option ids (empty = withdraw). `target` is the poll start.
    Vote { answers: Vec<String> },
    /// Close an own poll. `target` is the poll start.
    EndPoll,
}
#[derive(Clone, PartialEq, Eq)]
pub struct OperationRequest {
    pub scope: RoomScope,
    pub transaction_id: TransactionId,
    pub target: EventId,
    pub operation: Operation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationError {
    WrongScope,
    InvalidInput,
    Unavailable,
    Forbidden,
    Transport,
}

/// User search types live here, outside the `sdk` feature, because the desktop shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserHit {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    SameInstance,
    All,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchState {
    Idle,
    Searching,
    /// `direct` is the «Avvia con @nome:server» entry: set when the term is a valid user ID,
    /// even if the directory did not return it (then `hits` may be empty).
    Results {
        hits: Vec<UserHit>,
        direct: Option<String>,
    },
    NoResults,
    /// One line in the UI, no detail.
    Error,
}

/// Message search of one room («Cerca» in the chat header). Same rule as `SearchState`: the
/// desktop reads these, so they live outside the `sdk` feature. Terms and excerpts are private:
/// nothing logs them, and no `Debug` shows them in a trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageSource {
    Server,
    LocalCache,
}

#[derive(Clone, PartialEq, Eq)]
pub struct MessageHit {
    pub event_id: String,
    pub sender: String,
    /// Display name of the sender in the room, else the user ID.
    pub sender_name: String,
    /// `origin_server_ts` in ms.
    pub ts: u64,
    /// Body on one line (newlines become spaces), at most ~200 characters.
    pub excerpt: String,
    /// Byte ranges of the term inside `excerpt`, on char boundaries.
    pub matches: Vec<std::ops::Range<usize>>,
    /// `Some(root)` for a reply inside a thread.
    pub thread_root: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoreHits {
    None,
    Available,
    Loading,
    Failed,
}

#[derive(Clone, PartialEq, Eq)]
pub enum MessageSearchState {
    /// Term shorter than the minimum, or search closed.
    Idle,
    Searching { term: String },
    /// `partial`: encrypted room whose cache stops short of the room start (cap or no server).
    Results { term: String, source: MessageSource, hits: Vec<MessageHit>, more: MoreHits, partial: bool },
    NoResults { term: String, source: MessageSource },
    /// One line in the UI, no detail.
    Error { term: String },
}

/// Avatar of a new space: the image bytes and their MIME type, already read by the caller.
#[derive(Clone, PartialEq, Eq)]
pub struct SpaceAvatar {
    pub data: Vec<u8>,
    pub mime: String,
}

/// A space that exists but not exactly as asked; never fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceNotice {
    /// The server refused the directory listing (e.g. Synapse `room_list_publication_rules`).
    NotPublished,
    AvatarFailed(OperationError),
}

impl SpaceNotice {
    /// The one line the UI shows.
    pub fn line(self) -> &'static str {
        match self {
            Self::NotPublished => "Spazio creato, non pubblicato nella directory",
            Self::AvatarFailed(_) => "Spazio creato, immagine non caricata",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceCreated {
    pub scope: RoomScope,
    pub notices: Vec<SpaceNotice>,
}

/// A space as offered to the user for one room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceRef {
    pub scope: RoomScope,
    pub name: String,
    /// The user may send `m.space.child` there (read from the power levels, never assumed).
    pub editable: bool,
}

/// Spaces for one room, from `SpaceService`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpaceChoices {
    /// Spaces the user can edit that do not contain the room yet («Aggiungi a uno spazio…»).
    pub addable: Vec<SpaceRef>,
    /// Joined spaces that contain the room; «Togli da <spazio>» only for the `editable` ones.
    pub containing: Vec<SpaceRef>,
}

/// One direct child of a space, from the space hierarchy (rooms the user has not joined included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceChild {
    pub scope: RoomScope,
    pub name: String,
    pub avatar_url: Option<String>,
    pub joined: bool,
}

/// Children of one space plus whether the user may change them (`m.space.child` power level).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpaceChildren {
    pub children: Vec<SpaceChild>,
    pub editable: bool,
    /// The user is the last owner of the space (SDK leave handle); false when unknown.
    pub last_owner: bool,
    /// Joined and invited members besides the user, from the room summary (what «Elimina spazio» would kick).
    pub other_members: u64,
}

/// What «Elimina spazio» did; every step is tried on its own and reported, none is silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceDeleted {
    pub unlinked: usize,
    pub unlink_failed: usize,
    pub kicked: usize,
    /// Display names of the members that could not be kicked (or `None` when the member list was not readable).
    pub kick_failed: Option<Vec<String>>,
    /// `None` = the space was not left because members remain (the user can run it again).
    pub left: Option<Result<bool, OperationError>>,
}

/// The one line the UI shows when adding to or removing from a space failed.
pub fn space_error_line(error: OperationError) -> &'static str {
    match error {
        OperationError::Forbidden => "Non consentito dal server",
        OperationError::Unavailable | OperationError::WrongScope | OperationError::InvalidInput => "Spazio non trovato",
        OperationError::Transport => "Operazione non riuscita, riprova",
    }
}

pub enum OperationCommand {
    /// Spaces that can take / already hold `scope`; answered with `OperationResult::SpaceChoices`.
    SpaceChoices {
        scope: RoomScope,
    },
    /// Children of `scope` (a space) and the user's right to edit them; answered with `OperationResult::SpaceChildren`.
    SpaceChildren {
        scope: RoomScope,
    },
    /// Put `scope` under `space` (`m.space.child`; `m.space.parent` too when the room's levels allow it).
    /// Needs the right on the space only, not on the room. Answered with `OperationResult::SpaceChanged`.
    AddToSpace {
        scope: RoomScope,
        space: RoomScope,
    },
    /// Take `scope` out of `space`; answered with `OperationResult::SpaceChanged`.
    RemoveFromSpace {
        scope: RoomScope,
        space: RoomScope,
    },
    /// Search the messages of one room; an empty term closes the search. Answered with
    /// `OperationResult::MessageSearch` as it progresses.
    SearchMessages {
        scope: RoomScope,
        term: String,
    },
    /// Next page of the open message search.
    MoreMessages,
    Inspect {
        tag: crate::RequestTag,
        target: EventId,
    },
    Apply(OperationRequest),
    /// Joined members for the @ completion and the member list.
    Members {
        scope: RoomScope,
    },
    /// Global profile of another user (name, avatar), for the profile panel.
    Profile {
        user_id: String,
    },
    /// Room-level read marker from the room menu; `public` is the «Conferme di lettura» setting.
    MarkRoom {
        scope: RoomScope,
        read: bool,
        public: bool,
        /// Opening the room: receipt `main` only, so unread threads stay unread. The menu reads everything.
        main_only: bool,
    },
    /// New direct chat with `user_id` (reuses an existing one).
    StartDm {
        user_id: String,
    },
    /// New room; `public` joins by anyone and lists it in the directory.
    CreateRoom {
        name: String,
        topic: Option<String>,
        public: bool,
    },
    /// New space (`m.space`); `public` lists it in the directory, otherwise it is private and not encrypted.
    CreateSpace {
        name: String,
        topic: Option<String>,
        public: bool,
        avatar: Option<SpaceAvatar>,
    },
    /// «Modifica stanza»: only the parts that changed; avatar is an image file read here.
    EditRoom {
        scope: RoomScope,
        name: Option<String>,
        topic: Option<String>,
        avatar: Option<std::path::PathBuf>,
    },
    /// Indirizzi (alias) di una stanza: lettura e modifica, risposta `OperationResult::Aliases`.
    Aliases {
        scope: RoomScope,
        action: crate::alias::AliasAction,
    },
    /// Join by alias (`#a:server`) or room ID (`!id:server`), typed or taken from the directory.
    JoinRoom {
        target: String,
        /// Servers that can answer the join (`server_name`); empty = the target's own server.
        via: Vec<String>,
    },
    /// «Unisciti» search of a public room directory (`server` None = the account's own),
    /// answered with `OperationResult::RoomSearch` as it progresses. The term is never logged.
    SearchRooms {
        term: String,
        server: Option<String>,
        /// Only rooms or only spaces; with an empty `term` the directory's first page («Suggerimenti»).
        kind: Option<DirectoryKind>,
    },
    /// «Altri risultati» of the running room search.
    MoreRooms,
    /// «Aggiungi nuovo server…»: one directory request to see whether `server` answers.
    ProbeServer {
        server: String,
    },
    /// «Esci»: leave the joined room and forget it, so it drops out of the list.
    LeaveRoom {
        scope: RoomScope,
    },
    /// «Esci dallo spazio»: leaves only the space (never the rooms inside); `forget` = «Dimentica» switch.
    LeaveSpace {
        scope: RoomScope,
        forget: bool,
    },
    /// «Elimina spazio» (soft delete, Matrix has no real one): unlink the children, kick the members, leave and forget.
    DeleteSpace {
        scope: RoomScope,
    },
    /// Answer to an invitation, only on the user's click: accept joins, reject leaves (both SDK).
    AnswerInvite {
        scope: RoomScope,
        accept: bool,
    },
    /// «Nuova chat» search; answered with `OperationResult::UserSearch` as it progresses.
    SearchUsers {
        term: String,
        all: bool,
    },
    /// Group (room tag) change asked for by the user in «Modifica»; the only writer of tags.
    EditTags(TagEdit),
    /// The room's notification mode as an account push rule; the outcome comes back as
    /// `NotificationModeSet` and the next sync projects it.
    SetNotificationMode {
        scope: RoomScope,
        mode: matrix_domain::notifications::RoomNotifyMode,
    },
    /// Read-only thread panel: the root and the newest `want` replies (SDK thread timeline).
    /// The page proves nothing about the receipt; the panel asks for that separately with `ThreadRead`.
    Thread {
        tag: crate::RequestTag,
        root: EventId,
        want: usize,
    },
    /// The thread's read receipt on one explicit event the panel already shows. The worker polls it
    /// beside the commands (one in flight per account), so a frozen server never blocks a page.
    ThreadRead {
        tag: crate::RequestTag,
        root: EventId,
        target: EventId,
        /// «Conferme di lettura»: `m.read` when public, `m.read.private` otherwise. Never no receipt.
        public: bool,
    },
    /// Account-wide ignore (`ignore`) or unignore of `user_id`.
    Ignore { user_id: String, ignore: bool },
    /// What the current user may do to `user_id` in this room (moderation entries of the profile).
    MemberRights { scope: RoomScope, user_id: String },
    /// A moderation action on `user_id` in a room or space; answered with `OperationResult::Moderated`.
    Moderate { scope: RoomScope, user_id: String, action: Moderation },
    /// Members banned from the room or space; answered with `OperationResult::BannedMembers`.
    BannedMembers { scope: RoomScope },
    /// What the current user may do with members of the room or space; answered with `OperationResult::MemberPermissions`.
    MemberPermissions { scope: RoomScope },
    /// A text into the thread: a plain thread message, or a reply to `reply_to` (a thread event).
    ThreadSend {
        scope: RoomScope,
        root: EventId,
        reply_to: Option<EventId>,
        body: String,
    },
}
/// One explicit change of the personal groups. `tag` is the full tag name (`m.favourite`,
/// `m.lowpriority` or `u.<name>`); a group exists as long as one room carries its tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagEdit {
    Assign { room_id: String, tag: String, on: bool },
    /// Moves every room of user group `from` to `to` (both `u.*`).
    Rename { from: String, to: String },
    /// Removes user group `tag` from every room.
    Delete { tag: String },
}
pub enum OperationResult {
    MessageSearch {
        scope: RoomScope,
        state: MessageSearchState,
    },
    /// The server accepted the text; `Ok` carries the new event's ID.
    ThreadSent {
        scope: RoomScope,
        root: EventId,
        result: Result<EventId, OperationError>,
    },
    Thread {
        tag: crate::RequestTag,
        root: EventId,
        // Boxed: the page is far bigger than the other results.
        result: Result<Box<crate::history::ThreadPage>, OperationError>,
    },
    /// `Ok(true)` is the server ACK; `Ok(false)` is the SDK's no-op (no send, not a confirmed read).
    ThreadRead {
        tag: crate::RequestTag,
        root: EventId,
        target: EventId,
        result: Result<bool, OperationError>,
    },
    Profile {
        user_id: String,
        result: Result<UserProfile, OperationError>,
    },
    Ignored {
        user_id: String,
        ignore: bool,
        result: Result<(), OperationError>,
    },
    MemberRights {
        scope: RoomScope,
        user_id: String,
        result: Result<MemberRights, OperationError>,
    },
    BannedMembers {
        scope: RoomScope,
        result: Result<Vec<BannedMember>, OperationError>,
    },
    MemberPermissions {
        scope: RoomScope,
        result: Result<MemberPermissions, OperationError>,
    },
    Moderated {
        scope: RoomScope,
        user_id: String,
        action: Moderation,
        result: Result<(), OperationError>,
    },
    RoomSearch {
        state: crate::search::SearchState<DirectoryRoom>,
    },
    ServerProbed {
        server: String,
        result: Result<(), crate::search::SearchError>,
    },
    RoomLeft {
        scope: RoomScope,
        result: Result<(), OperationError>,
    },
    /// `is_last_owner`: the SDK handle's answer for the space (None when the handle was not obtained);
    /// `Ok(forgot)`: the leave is done or confirmed, `forgot` = `Room::forget` ran too.
    SpaceLeft {
        scope: RoomScope,
        is_last_owner: Option<bool>,
        result: Result<bool, OperationError>,
    },
    SpaceDeleted {
        scope: RoomScope,
        result: Result<SpaceDeleted, OperationError>,
    },
    UserSearch {
        state: SearchState,
        all_available: bool,
    },
    TagsEdited {
        result: Result<(), OperationError>,
    },
    RoomStarted {
        result: Result<RoomScope, OperationError>,
    },
    SpaceCreated {
        result: Result<SpaceCreated, OperationError>,
    },
    SpaceChoices {
        scope: RoomScope,
        result: Result<SpaceChoices, OperationError>,
    },
    SpaceChildren {
        scope: RoomScope,
        result: Result<SpaceChildren, OperationError>,
    },
    /// `added` tells which command it answers. The space list changes at the next sync.
    SpaceChanged {
        scope: RoomScope,
        space: RoomScope,
        added: bool,
        result: Result<(), OperationError>,
    },
    /// Progress of a `JoinRoom`; `target` is the command's string, echoed unchanged.
    Join {
        target: String,
        phase: crate::join::JoinPhase,
    },
    InviteAnswered {
        scope: RoomScope,
        accept: bool,
        result: Result<(), OperationError>,
    },
    RoomEdited {
        result: Result<(), OperationError>,
    },
    /// `aliases`: l'elenco riletto dal server (anche dopo un errore); `None` se la rilettura è fallita.
    Aliases {
        scope: RoomScope,
        action: crate::alias::AliasAction,
        result: Result<(), crate::alias::AliasOpError>,
        aliases: Option<crate::alias::RoomAliases>,
    },
    Members {
        scope: RoomScope,
        members: Result<Vec<Member>, OperationError>,
        /// The server's upload limit in bytes (media config); `None` when it could not be read.
        upload_limit: Option<u64>,
    },
    RoomMarked {
        scope: RoomScope,
        result: Result<(), OperationError>,
    },
    NotificationModeSet {
        scope: RoomScope,
        result: Result<(), OperationError>,
    },
    Inspected {
        tag: crate::RequestTag,
        target: EventId,
        details: Result<MessageDetails, OperationError>,
    },
    Applied {
        request: OperationRequest,
        result: Result<EventId, OperationError>,
    },
}

/// What a directory search lists: rooms (`room_types: [null]`) or spaces (`["m.space"]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryKind {
    Rooms,
    Spaces,
}

/// Some servers ignore the `room_types` filter: the answer is checked on each chunk's own type too.
#[cfg(feature = "sdk")]
pub fn kind_matches(kind: Option<DirectoryKind>, room_type: Option<&matrix_sdk::ruma::room::RoomType>) -> bool {
    use matrix_sdk::ruma::room::RoomType;
    match kind {
        None => true,
        Some(DirectoryKind::Spaces) => room_type == Some(&RoomType::Space),
        Some(DirectoryKind::Rooms) => room_type.is_none(),
    }
}

/// One public-directory hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryRoom {
    pub room_id: String,
    pub alias: Option<String>,
    pub name: String,
    pub topic: Option<String>,
    pub members: u64,
    pub avatar_url: Option<String>,
    pub world_readable: bool,
    pub guest_can_join: bool,
    /// `public`, `knock`, `invite`...: only `public` can be joined straight away.
    pub join_rule: String,
    /// Server whose directory listed the room; the `via` of the join.
    pub origin: String,
}
impl DirectoryRoom {
    /// What `JoinRoom` takes: the alias when the room has one, else the room ID.
    pub fn target(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.room_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub user_id: String,
    /// Display name, or the user id when the member has none.
    pub name: String,
    /// `mxc://` avatar of the member in this room (an empty string counts as none).
    pub avatar: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserProfile {
    pub user_id: String,
    /// Display name, or the user id when the profile has none.
    pub name: String,
    pub avatar: Option<String>,
    /// Cross-signing trust of the user; `None` when the SDK has no identity for them.
    pub verified: Option<bool>,
    /// On the account's ignore list (SDK account data).
    pub ignored: bool,
}

/// A member banned from a room or space (membership `ban`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannedMember {
    pub user_id: String,
    /// Display name, or the user id when the member has none.
    pub name: String,
    pub avatar: Option<String>,
    /// Reason of the ban event, when it has one.
    pub reason: Option<String>,
}

/// Room-wide member rights of the current user from the standard power levels (`invite`, `kick`, `ban`);
/// the UI hides the action when its flag is false. Unban needs `ban` (and `kick`) on top of the target's level:
/// `Moderate` with `Unban` re-checks it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemberPermissions {
    pub can_invite: bool,
    pub can_kick: bool,
    pub can_ban: bool,
}

/// Moderation of a room or space member. `Kick`/`Ban` carry no reason; `KickWith`/`BanWith`/`Unban` take one
/// (blank counts as none, so `Unban(None)` is a plain unban). `Invite` adds `user_id` to the room.
/// `RedactMessages` carries the exact events the user confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Moderation {
    Kick,
    Ban,
    KickWith(String),
    BanWith(String),
    Unban(Option<String>),
    Invite,
    SetPower(i64),
    RedactMessages(Vec<String>),
}

/// Power levels the profile offers; a custom level of the member shows only as «not one of these».
pub const POWER_LEVELS: [i64; 3] = [0, 50, 100];

/// What the current user may do to a joined member, per the room power levels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemberRights {
    /// Current role from the member's power level (also for oneself and non-members).
    pub role: matrix_domain::role::MemberRole,
    /// Privileged room creator (room v12+, infinite power level): read-only, nobody can change it.
    /// shortcut: creators of rooms before v12 are plain admins here; upgrade via `Room::creators()` if the UI needs them.
    pub creator: bool,
    /// The user may change this member's role: may edit power levels and the member ranks strictly below them.
    pub can_change_role: bool,
    pub kick: bool,
    pub ban: bool,
    /// Levels of `POWER_LEVELS` the user may give (at most their own, not the member's current one).
    pub levels: Vec<i64>,
    /// Loaded, not yet deleted messages of the member the user may delete; the confirmation counts these.
    pub recent: Vec<String>,
    /// Last event the member has read, only when that event is among the loaded ones.
    pub read_receipt: Option<String>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct MessagePermissions {
    pub reply: bool,
    pub edit: bool,
    pub redact: bool,
    pub react: bool,
}
pub struct MessageDetails {
    pub body: String,
    pub permissions: MessagePermissions,
}

#[cfg(feature = "sdk")]
const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The room's Timeline (the one the history uses) and the message item for `target`; pages back a
/// little when the message is not loaded yet.
#[cfg(feature = "sdk")]
async fn locate(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    target: &matrix_sdk::ruma::EventId,
    page: bool,
) -> Result<
    (
        matrix_sdk::Room,
        std::sync::Arc<matrix_sdk_ui::timeline::Timeline>,
        Option<matrix_sdk_ui::timeline::EventTimelineItem>,
    ),
    OperationError,
> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| OperationError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    let timeline = timelines
        .get(&room)
        .await
        .map_err(|_| OperationError::Unavailable)?;
    let mut item = timeline.item_by_event_id(target).await;
    // shortcut: about 3 s of polling (sync or pagination apply in the timeline's own task), one
    // page of 25 back every fifth step; then the message counts as not found.
    for step in 0..20 {
        if item.is_some() || !page {
            break;
        }
        if step % 5 == 4 {
            let _ = timeline.paginate_backwards(25).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        item = timeline.item_by_event_id(target).await;
    }
    Ok((room, timeline, item))
}

#[cfg(feature = "sdk")]
pub(crate) async fn details(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    target: &EventId,
) -> Result<MessageDetails, OperationError> {
    use matrix_sdk::ruma::events::MessageLikeEventType;
    let target = matrix_sdk::ruma::EventId::parse(target.as_str())
        .map_err(|_| OperationError::InvalidInput)?;
    tokio::time::timeout(WAIT, async {
        let (room, _, item) = locate(client, timelines, account, scope, &target, true).await?;
        let item = item.ok_or(OperationError::Unavailable)?;
        // A poll start is acted on like a message (EndPoll needs its redact right).
        let body = match (item.content().as_message(), item.content().as_poll()) {
            (Some(message), _) => message.body().to_owned(),
            (None, Some(poll)) => poll.results().question,
            _ => return Err(OperationError::Unavailable),
        };
        let user = client.user_id().ok_or(OperationError::Unavailable)?;
        // The SDK has no permission check: power levels stay ours.
        let levels = room
            .power_levels()
            .await
            .map_err(|_| OperationError::Unavailable)?;
        let send = levels.user_can_send_message(user, MessageLikeEventType::RoomMessage);
        Ok(MessageDetails {
            body,
            permissions: MessagePermissions {
                reply: send && item.can_be_replied_to(),
                edit: send && item.is_editable(),
                redact: if item.is_own() {
                    levels.user_can_redact_own_event(user)
                } else {
                    levels.user_can_redact_event_of_other(user)
                },
                react: levels.user_can_send_message(user, MessageLikeEventType::Reaction),
            },
        })
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

/// Read: receipts for the newest stored event, then the SDK unread flag off. Unread: the SDK unread flag on.
#[cfg(feature = "sdk")]
pub(crate) async fn mark_room(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    read: bool,
    public: bool,
    main_only: bool,
) -> Result<(), OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| OperationError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    if read {
        use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
        let timeline = timelines
            .get(&room)
            .await
            .map_err(|_| OperationError::Unavailable)?;
        let kind = if public { ReceiptType::Read } else { ReceiptType::ReadPrivate };
        if main_only {
            crate::threads::mark_main(&room, &timeline, public).await.map_err(|_| OperationError::Transport)?;
        } else {
            timeline.mark_as_read(kind).await.map_err(|_| OperationError::Transport)?;
        }
        timeline
            .mark_as_read(ReceiptType::FullyRead)
            .await
            .map_err(|_| OperationError::Transport)?;
    }
    room.set_unread_flag(!read)
        .await
        .map_err(|_| OperationError::Transport)
}

/// Direct chat: the SDK writes `m.direct`; an existing DM with the user is returned instead of a duplicate.
/// A user the server does not know stops here (`Unavailable`) so no orphan room is created.
#[cfg(feature = "sdk")]
pub async fn start_dm(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    user_id: &str,
) -> Result<RoomScope, OperationError> {
    let user = matrix_sdk::ruma::UserId::parse(user_id.trim())
        .map_err(|_| OperationError::InvalidInput)?;
    use matrix_sdk::ruma::events::room::member::MembershipState;
    // Reuse unless the store says B left/was banned: B's membership may not have synced yet (None/Err), which is not a reason for a second room.
    // get_dm_room() takes an arbitrary DM: after a declined invite there are two, so pick the first one B has not left.
    let mut reusable = None;
    for room in client.get_dm_rooms(&user) {
        let gone = matches!(room.get_member_no_sync(&user).await, Ok(Some(m)) if matches!(m.membership(), MembershipState::Leave | MembershipState::Ban));
        if !gone {
            reusable = Some(room);
            break;
        }
    }
    let room = match reusable {
        Some(room) => room,
        None => timeout_create(async {
            // Only NotFound stops: other profile failures (private profiles, network) fall through to create_dm.
            if let Err(e) = client.account().fetch_user_profile_of(&user).await
                && is_not_found(&e)
            {
                return Err(e);
            }
            client.create_dm(&user).await
        })
        .await?,
    };
    Ok(RoomScope {
        account_id: account,
        room_id: matrix_domain::RoomId::new(room.room_id().as_str()),
    })
}

/// Accept (`Room::join`: the server finds the inviting server from the invite, also over federation)
/// or reject (`Room::leave`) an invitation. Anything that is not an open invitation stops here.
#[cfg(feature = "sdk")]
pub async fn answer_invite(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    accept: bool,
) -> Result<(), OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| OperationError::WrongScope)?;
    let room = client.get_room(&id).ok_or(OperationError::Unavailable)?;
    if room.state() != matrix_sdk::RoomState::Invited {
        return Err(OperationError::Unavailable);
    }
    let answer = async { if accept { room.join().await } else { room.leave().await } };
    tokio::time::timeout(std::time::Duration::from_secs(30), answer)
        .await
        .map_err(|_| OperationError::Transport)?
        .map_err(|e| classify(&e))
}

/// Join by alias or room ID (`Client::join_room_by_id_or_alias`; the server resolves an alias).
/// `via` = servers to try (the directory server of a result); empty and a room ID: the ID's own
/// server. A room that needs an invite or a knock: the server says no, `Forbidden`.
#[cfg(feature = "sdk")]
pub async fn join_room(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    target: &str,
    via: &[String],
) -> Result<RoomScope, OperationError> {
    use matrix_sdk::ruma::OwnedRoomOrAliasId;
    let target = OwnedRoomOrAliasId::try_from(target.trim()).map_err(|_| OperationError::InvalidInput)?;
    let via = join_via(&target, via)?;
    let room = timeout_create(client.join_room_by_id_or_alias(&target, &via)).await?;
    Ok(RoomScope {
        account_id: account,
        room_id: matrix_domain::RoomId::new(room.room_id().as_str()),
    })
}

/// The `via` list of a join: the given servers (deduplicated, each a valid server name), else the
/// server of a room ID, else nothing (an alias resolves on its own server).
#[cfg(feature = "sdk")]
pub fn join_via(
    target: &matrix_sdk::ruma::OwnedRoomOrAliasId,
    via: &[String],
) -> Result<Vec<matrix_sdk::ruma::OwnedServerName>, OperationError> {
    let mut out: Vec<matrix_sdk::ruma::OwnedServerName> = Vec::new();
    for v in via {
        let v = matrix_sdk::ruma::OwnedServerName::try_from(v.trim()).map_err(|_| OperationError::InvalidInput)?;
        if !out.contains(&v) {
            out.push(v);
        }
    }
    if out.is_empty() && target.is_room_id() {
        out.extend(target.server_name().map(ToOwned::to_owned));
    }
    Ok(out)
}

/// A Matrix server name (`host`, `host:port`, `[v6]:port`) from the user's text; checked before any request.
#[cfg(feature = "sdk")]
pub fn parse_server(text: &str) -> Result<matrix_sdk::ruma::OwnedServerName, crate::search::SearchError> {
    matrix_sdk::ruma::OwnedServerName::try_from(text.trim()).map_err(|_| crate::search::SearchError::InvalidServer)
}

/// The POST /publicRooms body and query. `server` = another server's directory (our homeserver
/// relays); `None` or our own server name = the local directory, no `server` parameter.
#[cfg(feature = "sdk")]
pub fn directory_request(
    term: &str,
    server: Option<&str>,
    local: Option<&matrix_sdk::ruma::ServerName>,
    since: Option<String>,
    limit: u32,
    kind: Option<DirectoryKind>,
) -> Result<matrix_sdk::ruma::api::client::directory::get_public_rooms_filtered::v3::Request, crate::search::SearchError> {
    let mut request = matrix_sdk::ruma::api::client::directory::get_public_rooms_filtered::v3::Request::new();
    request.server = match server.map(parse_server).transpose()? {
        Some(s) if Some(&*s) != local => Some(s),
        _ => None,
    };
    request.limit = Some(limit.into());
    request.since = since;
    request.filter.generic_search_term = Some(term.trim().to_owned()).filter(|t| !t.is_empty());
    request.filter.room_types = match kind {
        None => Vec::new(),
        Some(DirectoryKind::Rooms) => vec![matrix_sdk::ruma::directory::RoomTypeFilter::Default],
        Some(DirectoryKind::Spaces) => vec![matrix_sdk::ruma::directory::RoomTypeFilter::Space],
    };
    Ok(request)
}

/// Directory answer to hits; `origin` is the server that was asked.
#[cfg(feature = "sdk")]
pub fn directory_page(
    response: matrix_sdk::ruma::api::client::directory::get_public_rooms_filtered::v3::Response,
    origin: &str,
) -> crate::search::Page<DirectoryRoom> {
    crate::search::Page {
        hits: response
            .chunk
            .into_iter()
            .map(|d| DirectoryRoom {
                room_id: d.room_id.to_string(),
                alias: d.canonical_alias.as_ref().map(ToString::to_string),
                name: d.name.clone().unwrap_or_else(|| d.canonical_alias.as_ref().map_or_else(|| d.room_id.to_string(), ToString::to_string)),
                topic: d.topic.filter(|t| !t.trim().is_empty()),
                members: d.num_joined_members.into(),
                avatar_url: d.avatar_url.map(|u| u.to_string()),
                world_readable: d.world_readable,
                guest_can_join: d.guest_can_join,
                join_rule: d.join_rule.as_str().to_owned(),
                origin: origin.to_owned(),
            })
            .collect(),
        next_batch: response.next_batch.filter(|b| !b.is_empty()),
    }
}

/// Transport failure = our homeserver is down; 400 = it rejected the server name; 401/403/404 =
/// the remote refuses its directory to us (many servers do, or do not federate it); the rest
/// (5xx relayed: the remote does not answer) = unreachable. No status but `answered` (our server replied,
/// the body was not a Matrix error, e.g. an unreadable 2xx) is unreachable too, not a connection failure.
#[cfg(feature = "sdk")]
pub fn directory_error(status: Option<u16>, kind: Option<&ErrorKind>, answered: bool) -> crate::search::SearchError {
    use crate::search::SearchError::*;
    match (status, kind) {
        (None, _) if answered => Unreachable,
        (None, _) => Network,
        (_, Some(ErrorKind::InvalidParam)) | (Some(400), _) => InvalidServer,
        (_, Some(ErrorKind::Forbidden | ErrorKind::Unrecognized | ErrorKind::NotFound)) => Refused,
        (Some(401 | 403 | 404), _) => Refused,
        _ => Unreachable,
    }
}

/// One page of a public room directory (`Client::public_rooms_filtered`, POST /publicRooms), `term`
/// matched by the server. No debounce or timeout here: the driver aborts the future when superseded.
/// Privacy: the term is never logged.
#[cfg(feature = "sdk")]
pub async fn public_rooms(
    client: &matrix_sdk::Client,
    term: &str,
    server: Option<&str>,
    since: Option<String>,
    limit: u32,
    kind: Option<DirectoryKind>,
) -> Result<crate::search::Page<DirectoryRoom>, crate::search::SearchError> {
    let local = client.user_id().map(|u| u.server_name().to_owned());
    let request = directory_request(term, server, local.as_deref(), since, limit, kind)?;
    let origin = request.server.as_ref().map(ToString::to_string).or_else(|| local.map(|l| l.to_string())).unwrap_or_default();
    // The SDK retries 5xx; a home server's 502 for a dead remote must surface at once, not as a 20 s timeout.
    let response = client.send(request).with_request_config(matrix_sdk::config::RequestConfig::new().disable_retry()).await.map_err(|e| {
        let api = e.as_client_api_error();
        directory_error(api.map(|a| a.status_code.as_u16()), e.client_api_error_kind(), matches!(e, matrix_sdk::HttpError::Api(_)))
    })?;
    let mut response = response;
    response.chunk.retain(|d| kind_matches(kind, d.room_type.as_ref()));
    Ok(directory_page(response, &origin))
}

/// Leave a joined room (`Room::leave`). No `forget`: the room stays in the store as Left (history kept) and the
/// list projection skips Left rooms, also after a restart.
/// SDK 0.19.1 `leave_impl` treats M_FORBIDDEN as a successful leave, so a refused leave would look done:
/// after `Ok` the server is asked for our own membership: `join` or an unreadable answer becomes `Forbidden`.
#[cfg(feature = "sdk")]
pub async fn leave_room(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
) -> Result<(), OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| OperationError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    let left = tokio::time::timeout(std::time::Duration::from_secs(30), room.leave())
        .await
        .map_err(|_| OperationError::Transport)?
        .map_err(|e| classify(&e));
    left_or_refused(client, &id, left).await
}

/// The vendored SDK returns Forbidden without touching the local state; only the server decides
/// whether we are still in: already out (kicked/left elsewhere) counts as a completed leave.
#[cfg(feature = "sdk")]
async fn left_or_refused(
    client: &matrix_sdk::Client,
    id: &matrix_sdk::ruma::RoomId,
    left: Result<(), OperationError>,
) -> Result<(), OperationError> {
    match left {
        Err(e) if e != OperationError::Forbidden => Err(e),
        _ => match membership(client, id).await {
            Membership::NotJoined => Ok(()),
            // Still in, or unreadable: the row must stay, with the error line.
            Membership::Joined | Membership::Unknown => Err(OperationError::Forbidden),
        },
    }
}

/// SDK leave handle of a joined space (also tells whether we are its last owner); shared by the check and the leave.
#[cfg(feature = "sdk")]
async fn leave_handle(
    client: &matrix_sdk::Client,
    id: &matrix_sdk::ruma::RoomId,
) -> Result<matrix_sdk_ui::spaces::leave::LeaveSpaceHandle, OperationError> {
    // The space graph also holds the plain rooms a space lists as children: only a space may be left here.
    if !client.get_room(id).is_some_and(|r| r.is_space()) {
        return Err(OperationError::Unavailable);
    }
    let service = matrix_sdk_ui::spaces::SpaceService::new(client.clone()).await;
    service.top_level_joined_spaces().await; // fills the space graph, as in `space_choices`
    match tokio::time::timeout(std::time::Duration::from_secs(20), service.leave_space(id)).await {
        Ok(Ok(handle)) => Ok(handle),
        Ok(Err(e)) => Err(classify_space(&e)),
        Err(_) => Err(OperationError::Transport),
    }
}

/// «Esci dallo spazio»: `SpaceService::leave_space` → handle (whose `is_last_owner` is returned for the
/// confirmation text) → `handle.leave` filtered to the space itself, so the contained rooms stay joined.
/// Same already-out recheck as `leave_room` (Synapse 403, Tuwunel 200). `Room::forget` only after a leave
/// that succeeded or was confirmed by the recheck; a failing forget does not undo the leave (`Ok(false)`).
#[cfg(feature = "sdk")]
pub async fn leave_space(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    forget: bool,
) -> (Option<bool>, Result<bool, OperationError>) {
    let id = match room_id_of(account, scope) {
        Ok(id) => id,
        Err(e) => return (None, Err(e)),
    };
    // Already out (left elsewhere): nothing to leave, only the forget remains.
    let already_out = client.get_room(&id).is_some_and(|r| r.is_space() && r.state() == matrix_sdk::RoomState::Left);
    let mut is_last_owner = None;
    if !already_out {
        let handle = match leave_handle(client, &id).await {
            Ok(handle) => handle,
            Err(e) => return (None, Err(e)),
        };
        is_last_owner = handle.rooms().iter().find(|r| r.space_room.room_id == id).map(|r| r.is_last_owner);
        let left = match tokio::time::timeout(std::time::Duration::from_secs(30), handle.leave(|r| r.space_room.room_id == id)).await {
            Ok(left) => left.map_err(|e| classify_space(&e)),
            Err(_) => Err(OperationError::Transport),
        };
        if let Err(e) = left_or_refused(client, &id, left).await {
            return (is_last_owner, Err(e));
        }
    }
    let forgot = match client.get_room(&id).filter(|_| forget) {
        Some(room) => room.forget().await.map_err(|e| tracing::warn!(%e, "forget after leaving a space failed")).is_ok(),
        None => false,
    };
    (is_last_owner, Ok(forgot))
}

/// «Elimina spazio»: only for the last owner who may edit the space (rechecked here). Standard calls only:
/// `remove_from_space` for every child (the rooms stay), a kick for every other member, then `leave_space` with
/// forget. A failed step is counted, not fatal; with members left over the space is NOT left (no orphan without admin).
#[cfg(feature = "sdk")]
pub async fn delete_space(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
) -> Result<SpaceDeleted, OperationError> {
    use matrix_sdk::RoomMemberships;
    let facts = space_children(client, account, scope).await?;
    if !facts.last_owner || !facts.editable {
        return Err(OperationError::Forbidden);
    }
    let (mut unlinked, mut unlink_failed) = (0, 0);
    for child in &facts.children {
        match remove_from_space(client, account, &child.scope, scope).await {
            Ok(()) => unlinked += 1,
            Err(_) => unlink_failed += 1,
        }
    }
    let me = client.user_id().ok_or(OperationError::Unavailable)?;
    let room = client.get_room(&room_id_of(account, scope)?).filter(|r| r.state() == matrix_sdk::RoomState::Joined).ok_or(OperationError::Unavailable)?;
    let people = tokio::time::timeout(std::time::Duration::from_secs(15), room.members(RoomMemberships::JOIN | RoomMemberships::INVITE)).await;
    let (mut kicked, mut failed) = (0, Vec::new());
    let kick_failed = match people {
        Ok(Ok(people)) => {
            for m in people.iter().filter(|m| m.user_id() != me) {
                match moderate(client, account, scope, m.user_id().as_str(), &Moderation::Kick).await {
                    Ok(()) => kicked += 1,
                    Err(_) => failed.push(m.display_name().map_or_else(|| m.user_id().to_string(), str::to_owned)),
                }
            }
            Some(failed)
        }
        _ => None,
    };
    let left = if kick_failed.as_ref().is_some_and(Vec::is_empty) { Some(leave_space(client, account, scope, true).await.1) } else { None };
    Ok(SpaceDeleted { unlinked, unlink_failed, kicked, kick_failed, left })
}

#[cfg(feature = "sdk")]
pub(crate) enum Membership {
    Joined,
    NotJoined,
    Unknown,
}

/// Server-side check of our own `m.room.member` content. Only an answer proves we are out (a membership that
/// is not `join`, or 404: no member state); any other failure to ask is `Unknown`, never «not joined».
#[cfg(feature = "sdk")]
pub(crate) async fn membership(client: &matrix_sdk::Client, room: &matrix_sdk::ruma::RoomId) -> Membership {
    use matrix_sdk::ruma::{api::client::state::get_state_event_for_key::v3::Request, events::StateEventType};
    let Some(me) = client.user_id() else { return Membership::Unknown };
    let request = Request::new(room.to_owned(), StateEventType::RoomMember, me.to_string());
    match tokio::time::timeout(std::time::Duration::from_secs(10), client.send(request)).await {
        Ok(Ok(r)) if content_is_join(r.event_or_content.get()) => Membership::Joined,
        Ok(Ok(_)) => Membership::NotJoined,
        Ok(Err(e)) if e.as_client_api_error().is_some_and(|a| a.status_code.as_u16() == 404) => Membership::NotJoined,
        _ => Membership::Unknown,
    }
}

#[cfg(feature = "sdk")]
fn content_is_join(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json).is_ok_and(|v| v["membership"] == "join")
}

/// Name, topic and avatar through the SDK state-event setters; the server decides on permissions.
#[cfg(feature = "sdk")]
pub async fn edit_room(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    name: Option<&str>,
    topic: Option<&str>,
    avatar: Option<&std::path::Path>,
) -> Result<(), OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| OperationError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    let name = name.map(str::trim);
    if name == Some("") {
        return Err(OperationError::InvalidInput);
    }
    if let Some(name) = name {
        within(room.set_name(name.to_owned())).await?;
    }
    if let Some(topic) = topic {
        within(room.set_room_topic(topic.trim_end())).await?;
    }
    if let Some(path) = avatar {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        if mime.type_() != mime_guess::mime::IMAGE {
            return Err(OperationError::InvalidInput);
        }
        let data = tokio::fs::read(path).await.map_err(|_| OperationError::InvalidInput)?;
        within(room.upload_avatar(&mime, data, None)).await?;
    }
    Ok(())
}

#[cfg(feature = "sdk")]
async fn within<T>(call: impl std::future::Future<Output = matrix_sdk::Result<T>>) -> Result<(), OperationError> {
    tokio::time::timeout(std::time::Duration::from_secs(20), call)
        .await
        .map_err(|_| OperationError::Transport)?
        .map(drop)
        .map_err(|e| classify(&e))
}

/// Private rooms are born encrypted (like the SDK's DMs); public ones are not.
#[cfg(feature = "sdk")]
pub async fn create_room(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    name: &str,
    topic: Option<&str>,
    public: bool,
) -> Result<RoomScope, OperationError> {
    use matrix_sdk::ruma::{
        api::client::room::{
            Visibility,
            create_room::v3::{Request, RoomPreset},
        },
        events::{InitialStateEvent, room::encryption::RoomEncryptionEventContent},
    };
    let name = name.trim();
    if name.is_empty() {
        return Err(OperationError::InvalidInput);
    }
    let mut request = Request::new();
    request.name = Some(name.to_owned());
    request.topic = topic.map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
    (request.preset, request.visibility) = if public {
        (Some(RoomPreset::PublicChat), Visibility::Public)
    } else {
        request.initial_state = vec![
            InitialStateEvent::with_empty_state_key(
                RoomEncryptionEventContent::with_recommended_defaults(),
            )
            .to_raw_any(),
        ];
        (Some(RoomPreset::PrivateChat), Visibility::Private)
    };
    let room = timeout_create(client.create_room(request)).await?;
    Ok(RoomScope {
        account_id: account,
        room_id: matrix_domain::RoomId::new(room.room_id().as_str()),
    })
}

/// A space has no messages, so unlike `create_room` it never gets `m.room.encryption`.
/// Public ones are created private and listed afterwards, so a refused listing leaves a space, not an error.
#[cfg(feature = "sdk")]
fn space_request(name: &str, topic: Option<&str>, public: bool) -> matrix_sdk::ruma::api::client::room::create_room::v3::Request {
    use matrix_sdk::ruma::{
        api::client::room::{
            Visibility,
            create_room::v3::{CreationContent, Request, RoomPreset},
        },
        room::RoomType,
        serde::Raw,
    };
    let mut creation = CreationContent::new();
    creation.room_type = Some(RoomType::Space);
    let mut request = Request::new();
    request.name = Some(name.to_owned());
    request.topic = topic.map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
    request.creation_content = Raw::new(&creation).ok();
    request.visibility = Visibility::Private;
    request.preset = Some(if public { RoomPreset::PublicChat } else { RoomPreset::PrivateChat });
    request
}

#[cfg(feature = "sdk")]
pub async fn create_space(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    name: &str,
    topic: Option<&str>,
    public: bool,
    avatar: Option<&SpaceAvatar>,
) -> Result<SpaceCreated, OperationError> {
    let name = name.trim();
    let mime = match avatar {
        Some(a) => Some(a.mime.parse::<mime_guess::mime::Mime>().map_err(|_| OperationError::InvalidInput)?),
        None => None,
    };
    if name.is_empty() || mime.as_ref().is_some_and(|m| m.type_() != mime_guess::mime::IMAGE) {
        return Err(OperationError::InvalidInput);
    }
    let room = timeout_create(client.create_room(space_request(name, topic, public))).await?;
    let mut notices = Vec::new();
    // Any refusal or timeout of the listing is the same outcome for the user: created, not listed.
    if public
        && within(room.privacy_settings().update_room_visibility(matrix_sdk::ruma::api::client::room::Visibility::Public))
            .await
            .is_err()
    {
        notices.push(SpaceNotice::NotPublished);
    }
    if let (Some(a), Some(mime)) = (avatar, mime.as_ref())
        && let Err(e) = within(room.upload_avatar(mime, a.data.clone(), None)).await
    {
        notices.push(SpaceNotice::AvatarFailed(e));
    }
    Ok(SpaceCreated {
        scope: RoomScope {
            account_id: account,
            room_id: matrix_domain::RoomId::new(room.room_id().as_str()),
        },
        notices,
    })
}

#[cfg(feature = "sdk")]
use matrix_sdk::ruma::api::error::ErrorKind;

/// Synapse answers 404 `M_UNKNOWN` ("No row found") for a missing profile, Tuwunel `M_NOT_FOUND`.
#[cfg(feature = "sdk")]
fn is_not_found(error: &matrix_sdk::Error) -> bool {
    matches!(error.client_api_error_kind(), Some(ErrorKind::NotFound))
        || error.as_client_api_error().is_some_and(|e| e.status_code == 404)
}

/// Server answer by Matrix error code; no client API error means the network failed.
#[cfg(feature = "sdk")]
fn classify(error: &matrix_sdk::Error) -> OperationError {
    if is_not_found(error) {
        OperationError::Unavailable
    } else if matches!(error.client_api_error_kind(), Some(ErrorKind::Forbidden)) {
        OperationError::Forbidden
    } else {
        OperationError::Transport
    }
}

#[cfg(feature = "sdk")]
async fn timeout_create(
    create: impl std::future::Future<Output = matrix_sdk::Result<matrix_sdk::Room>>,
) -> Result<matrix_sdk::Room, OperationError> {
    tokio::time::timeout(std::time::Duration::from_secs(20), create)
        .await
        .map_err(|_| OperationError::Transport)?
        .map_err(|e| classify(&e))
}

#[cfg(feature = "sdk")]
fn room_id_of(account: matrix_domain::AccountId, scope: &RoomScope) -> Result<matrix_sdk::ruma::OwnedRoomId, OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)
}

/// Error of a `SpaceService` call by what the server said (403 stays distinct from a network fault).
#[cfg(feature = "sdk")]
fn classify_space(error: &matrix_sdk_ui::spaces::Error) -> OperationError {
    use matrix_sdk_ui::spaces::Error as E;
    match error {
        E::UpdateRelationship(e) | E::UpdateInverseRelationship(e) | E::LeaveSpace(e) | E::LoadRoomMembers(e) => classify(e),
        E::RoomNotFound(_) | E::MissingState(..) => OperationError::Unavailable,
        E::UserIdNotFound => OperationError::Transport,
    }
}

/// One `SpaceService` write with the usual 20 s limit. A failed `m.space.parent` after the `m.space.child`
/// change is not an error: the parent link is optional and the change the user asked for is done.
#[cfg(feature = "sdk")]
async fn space_write(
    call: impl std::future::Future<Output = Result<(), matrix_sdk_ui::spaces::Error>>,
) -> Result<(), OperationError> {
    match tokio::time::timeout(std::time::Duration::from_secs(20), call)
        .await
        .map_err(|_| OperationError::Transport)?
    {
        Err(matrix_sdk_ui::spaces::Error::UpdateInverseRelationship(_)) | Ok(()) => Ok(()),
        Err(e) => Err(classify_space(&e)),
    }
}

/// Puts `room` under `space` with `SpaceService::add_child_to_space`, which writes `m.space.parent` only when
/// the room's own power levels allow it. Only the right on the space is needed (not admin of the room).
/// Idempotent: adding twice rewrites the same state.
#[cfg(feature = "sdk")]
pub async fn add_to_space(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    room: &RoomScope,
    space: &RoomScope,
) -> Result<(), OperationError> {
    let (room, space) = (room_id_of(account, room)?, room_id_of(account, space)?);
    let service = matrix_sdk_ui::spaces::SpaceService::new(client.clone()).await;
    space_write(service.add_child_to_space(room, space)).await
}

/// Takes `room` out of `space` with `SpaceService::remove_child_from_space` (an empty `m.space.child`,
/// and an empty `m.space.parent` when allowed). A room that is not in the space is a no-op.
#[cfg(feature = "sdk")]
pub async fn remove_from_space(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    room: &RoomScope,
    space: &RoomScope,
) -> Result<(), OperationError> {
    let (room, space) = (room_id_of(account, room)?, room_id_of(account, space)?);
    let service = matrix_sdk_ui::spaces::SpaceService::new(client.clone()).await;
    space_write(service.remove_child_from_space(room, space)).await
}

/// Whether the user may send `m.space.child` in `space`, from its power levels (the defaults differ per server).
#[cfg(feature = "sdk")]
pub async fn can_edit_space(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    space: &RoomScope,
) -> Result<bool, OperationError> {
    let id = room_id_of(account, space)?;
    let me = client.user_id().ok_or(OperationError::Unavailable)?;
    let room = client.get_room(&id).filter(|r| r.state() == matrix_sdk::RoomState::Joined).ok_or(OperationError::Unavailable)?;
    let levels = room.power_levels().await.map_err(|_| OperationError::Unavailable)?;
    Ok(levels.user_can_send_state(me, matrix_sdk::ruma::events::StateEventType::SpaceChild))
}

/// Direct children of `space` from the server hierarchy (all pages), with the edit right from its power levels.
#[cfg(feature = "sdk")]
pub async fn space_children(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    space: &RoomScope,
) -> Result<SpaceChildren, OperationError> {
    use matrix_sdk_ui::spaces::room_list::SpaceRoomListPaginationState::Idle;
    let id = room_id_of(account, space)?;
    let editable = can_edit_space(client, account, space).await?;
    let last_owner = match leave_handle(client, &id).await {
        Ok(h) => h.rooms().iter().any(|r| r.space_room.room_id == id && r.is_last_owner),
        Err(_) => false,
    };
    let service = matrix_sdk_ui::spaces::SpaceService::new(client.clone()).await;
    let list = service.space_room_list(id.clone()).await;
    // Bounded: a huge space stops at 50 pages rather than holding the command task.
    for _ in 0..50 {
        tokio::time::timeout(std::time::Duration::from_secs(20), list.paginate())
            .await
            .map_err(|_| OperationError::Transport)?
            .map_err(|e| classify(&e))?;
        if matches!(list.pagination_state(), Idle { end_reached: true }) {
            break;
        }
    }
    let children = list
        .rooms()
        .await
        .into_iter()
        .map(|r| SpaceChild {
            scope: RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(r.room_id.as_str()) },
            name: r.display_name,
            avatar_url: r.avatar_url.map(|u| u.to_string()),
            joined: r.state == Some(matrix_sdk::RoomState::Joined),
        })
        .collect();
    // The member list, not the sync summary counts (Synapse leaves them stale for a while); unknown reads as 0.
    let other_members = match client.get_room(&id) {
        Some(r) => tokio::time::timeout(std::time::Duration::from_secs(10), r.members(matrix_sdk::RoomMemberships::JOIN | matrix_sdk::RoomMemberships::INVITE))
            .await
            .ok()
            .and_then(Result::ok)
            .map_or(0, |m| m.iter().filter(|m| Some(m.user_id()) != client.user_id()).count() as u64),
        None => 0,
    };
    Ok(SpaceChildren { children, editable, last_owner, other_members })
}

/// Spaces that can take `room` and spaces that already hold it. The SDK graph is rebuilt on each call, so
/// the answer follows the last sync; a built `SpaceService` is not kept because the command task has no `RoomSync`.
#[cfg(feature = "sdk")]
pub async fn space_choices(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    room: &RoomScope,
) -> Result<SpaceChoices, OperationError> {
    let id = room_id_of(account, room)?;
    let service = matrix_sdk_ui::spaces::SpaceService::new(client.clone()).await;
    service.top_level_joined_spaces().await;
    let editable = service.editable_spaces().await;
    let parents = service.joined_parent_ids_of_child(&id).await;
    let of = |room_id: &matrix_sdk::ruma::RoomId, name: String, editable: bool| SpaceRef {
        scope: RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(room_id.as_str()) },
        name,
        editable,
    };
    let addable = editable
        .iter()
        .filter(|s| s.room_id != id && !parents.contains(&s.room_id))
        .map(|s| of(&s.room_id, s.display_name.clone(), true))
        .collect();
    let mut containing = Vec::new();
    for parent in &parents {
        if let Some(space) = service.get_space_room(parent).await {
            containing.push(of(parent, space.display_name, editable.iter().any(|e| &e.room_id == parent)));
        }
    }
    Ok(SpaceChoices { addable, containing })
}

#[cfg(feature = "sdk")]
pub async fn members(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
) -> Result<Vec<Member>, OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| OperationError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    // Lazy loading leaves the list partial until the full membership is fetched.
    let joined = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        room.members(matrix_sdk::RoomMemberships::JOIN),
    )
    .await
    .map_err(|_| OperationError::Transport)?
    .map_err(|_| OperationError::Transport)?;
    Ok(joined
        .into_iter()
        .map(|m| Member {
            user_id: m.user_id().to_string(),
            name: m
                .display_name()
                .map_or_else(|| m.user_id().to_string(), str::to_owned),
            avatar: m.avatar_url().map(ToString::to_string).filter(|u| !u.is_empty()),
        })
        .collect())
}

/// Profile of `user_id` from the homeserver (`Account::fetch_user_profile_of`).
#[cfg(feature = "sdk")]
pub async fn user_profile(
    client: &matrix_sdk::Client,
    user_id: &str,
) -> Result<UserProfile, OperationError> {
    use matrix_sdk::ruma::api::client::profile::{AvatarUrl, DisplayName};
    let id = matrix_sdk::ruma::UserId::parse(user_id.trim()).map_err(|_| OperationError::InvalidInput)?;
    let fetched = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        client.account().fetch_user_profile_of(&id),
    )
    .await
    .map_err(|_| OperationError::Transport)?;
    let profile = match fetched {
        Ok(p) => p,
        // Synapse answers 404 for a user with no display name and no avatar: a user we share a room with is an empty profile, not a missing one.
        Err(e) if is_not_found(&e) && shares_room_with(client, &id).await => Default::default(),
        Err(e) => return Err(classify(&e)),
    };
    Ok(UserProfile {
        user_id: id.to_string(),
        name: profile
            .get_static::<DisplayName>()
            .ok()
            .flatten()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.to_string()),
        avatar: profile.get_static::<AvatarUrl>().ok().flatten().map(|u| u.to_string()).filter(|u| !u.is_empty()),
        verified: user_identity(client, &id).await.map(|i| i.is_verified()),
        ignored: is_ignored(client, &id).await,
    })
}

#[cfg(feature = "sdk")]
async fn shares_room_with(client: &matrix_sdk::Client, id: &matrix_sdk::ruma::UserId) -> bool {
    for room in client.joined_rooms() {
        if matches!(room.get_member_no_sync(id).await, Ok(Some(_))) {
            return true;
        }
    }
    false
}

#[cfg(feature = "sdk")]
async fn is_ignored(client: &matrix_sdk::Client, id: &matrix_sdk::ruma::UserId) -> bool {
    use matrix_sdk::ruma::events::ignored_user_list::IgnoredUserListEventContent;
    let list = client.account().account_data::<IgnoredUserListEventContent>().await;
    list.ok().flatten().and_then(|raw| raw.deserialize().ok()).is_some_and(|c| c.ignored_users.contains_key(id))
}

/// Adds `user_id` to the account's ignore list, or removes it (`Account::ignore_user` / `unignore_user`).
#[cfg(feature = "sdk")]
pub async fn set_ignored(client: &matrix_sdk::Client, user_id: &str, ignore: bool) -> Result<(), OperationError> {
    let id = matrix_sdk::ruma::UserId::parse(user_id.trim()).map_err(|_| OperationError::InvalidInput)?;
    let account = client.account();
    tokio::time::timeout(WAIT, async {
        if ignore { account.ignore_user(&id).await } else { account.unignore_user(&id).await }.map_err(|e| classify(&e))
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

/// E2EE identity of `id`: the local store first, else one `/keys/query` (same on Synapse and Tuwunel).
#[cfg(feature = "sdk")]
pub(crate) async fn user_identity(
    client: &matrix_sdk::Client,
    id: &matrix_sdk::ruma::UserId,
) -> Option<matrix_sdk::encryption::identities::UserIdentity> {
    let encryption = client.encryption();
    if let Ok(Some(identity)) = encryption.get_user_identity(id).await {
        return Some(identity);
    }
    tokio::time::timeout(WAIT, encryption.request_user_identity(id))
        .await
        .ok()?
        .ok()?
}

/// Rights of `me` over `target` from the room power levels (SDK checks stay with ruma). The bool is
/// whether `me` may delete the messages of others. Nothing for oneself or a non-member.
#[cfg(feature = "sdk")]
fn member_rights(
    pl: &matrix_sdk::ruma::events::room::power_levels::RoomPowerLevels,
    me: &matrix_sdk::ruma::UserId,
    target: &matrix_sdk::ruma::UserId,
    joined: bool,
) -> (MemberRights, bool) {
    use matrix_domain::role::MemberRole;
    use matrix_sdk::ruma::{Int, events::room::power_levels::UserPowerLevel};
    let (mine, theirs) = (pl.for_user(me), pl.for_user(target));
    let (role, creator) = match theirs {
        UserPowerLevel::Int(i) => (MemberRole::from_power_level(i64::from(i)), false),
        _ => (MemberRole::Admin, true), // non-exhaustive enum: the only other value is Infinite
    };
    if !joined || me == target {
        return (MemberRights { role, creator, ..Default::default() }, false);
    }
    let levels = if pl.user_can_change_user_power_level(me, target) {
        POWER_LEVELS
            .into_iter()
            .filter(|l| {
                let l = UserPowerLevel::Int(Int::from(*l as i32));
                l <= mine && l != theirs
            })
            .collect()
    } else {
        Vec::new()
    };
    let rights = MemberRights {
        role,
        creator,
        can_change_role: !levels.is_empty(),
        kick: pl.user_can_kick_user(me, target),
        ban: pl.user_can_ban_user(me, target),
        levels,
        recent: Vec::new(),
        read_receipt: None,
    };
    (rights, pl.user_can_redact_event_of_other(me))
}

#[cfg(feature = "sdk")]
async fn moderation_room(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    user_id: &str,
) -> Result<(matrix_sdk::Room, matrix_sdk::ruma::OwnedUserId), OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let target = matrix_sdk::ruma::UserId::parse(user_id.trim()).map_err(|_| OperationError::InvalidInput)?;
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    Ok((room, target))
}

#[cfg(feature = "sdk")]
pub(crate) async fn load_member_rights(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    user_id: &str,
) -> Result<MemberRights, OperationError> {
    use matrix_sdk_ui::timeline::{MsgLikeKind, TimelineItemContent};
    let (room, target) = moderation_room(client, account, scope, user_id).await?;
    let me = client.user_id().ok_or(OperationError::Unavailable)?;
    tokio::time::timeout(WAIT, async {
        let pl = room.power_levels().await.map_err(|_| OperationError::Unavailable)?;
        let joined = room
            .get_member(&target)
            .await
            .ok()
            .flatten()
            .is_some_and(|m| matches!(m.membership(), matrix_sdk::ruma::events::room::member::MembershipState::Join));
        let (mut rights, redact) = member_rights(&pl, me, &target, joined);
        if joined && me != target {
            let timeline = timelines.get(&room).await.map_err(|_| OperationError::Unavailable)?;
            if let Some((id, _)) = timeline.latest_user_read_receipt(&target).await
                && timeline.item_by_event_id(&id).await.is_some()
            {
                rights.read_receipt = Some(id.to_string());
            }
            if redact {
                // shortcut: only messages the timeline has loaded count as «recent»; upgrade by paginating if wanted.
                rights.recent = timeline
                    .items()
                    .await
                    .iter()
                    .filter_map(|i| i.as_event())
                    .filter(|e| e.sender() == target && matches!(e.content(), TimelineItemContent::MsgLike(m) if !matches!(m.kind, MsgLikeKind::Redacted))) // real messages only: membership/profile/state events must never be redacted
                    .filter_map(|e| e.event_id().map(ToString::to_string))
                    .collect();
            }
        }
        Ok(rights)
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

fn reason(r: Option<&String>) -> Option<&str> {
    r.map(|r| r.trim()).filter(|r| !r.is_empty())
}

/// Members with membership `ban` in the room or space, each with the reason of its ban event.
#[cfg(feature = "sdk")]
pub async fn banned_members(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
) -> Result<Vec<BannedMember>, OperationError> {
    let id = room_id_of(account, scope)?;
    let room = client.get_room(&id).filter(|r| r.state() == matrix_sdk::RoomState::Joined).ok_or(OperationError::Unavailable)?;
    let members = tokio::time::timeout(WAIT, room.members(matrix_sdk::RoomMemberships::BAN))
        .await
        .map_err(|_| OperationError::Transport)?
        .map_err(|e| classify(&e))?;
    Ok(members
        .iter()
        .map(|m| BannedMember {
            user_id: m.user_id().to_string(),
            name: m.display_name().filter(|n| !n.is_empty()).unwrap_or(m.user_id().as_str()).to_string(),
            avatar: m.avatar_url().map(|u| u.to_string()).filter(|u| !u.is_empty()),
            reason: m.event().reason().map(str::trim).filter(|r| !r.is_empty()).map(str::to_string),
        })
        .collect())
}

/// Invite/kick/ban rights of the current user in the room or space, from its power levels.
#[cfg(feature = "sdk")]
pub async fn member_permissions(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
) -> Result<MemberPermissions, OperationError> {
    let id = room_id_of(account, scope)?;
    let me = client.user_id().ok_or(OperationError::Unavailable)?;
    let room = client.get_room(&id).filter(|r| r.state() == matrix_sdk::RoomState::Joined).ok_or(OperationError::Unavailable)?;
    let pl = room.power_levels().await.map_err(|_| OperationError::Unavailable)?;
    Ok(MemberPermissions { can_invite: pl.user_can_invite(me), can_kick: pl.user_can_kick(me), can_ban: pl.user_can_ban(me) })
}

/// Re-checks the rights at the moment of the action (the levels may have changed since the panel loaded),
/// then runs it through the SDK. Redaction deletes exactly the events given, nothing else.
#[cfg(feature = "sdk")]
pub(crate) async fn moderate(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    user_id: &str,
    action: &Moderation,
) -> Result<(), OperationError> {
    let (room, target) = moderation_room(client, account, scope, user_id).await?;
    let me = client.user_id().ok_or(OperationError::Unavailable)?;
    let pl = room.power_levels().await.map_err(|_| OperationError::Unavailable)?;
    let (rights, redact) = member_rights(&pl, me, &target, true);
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        match action {
            Moderation::Kick if rights.kick => room.kick_user(&target, None).await.map_err(|e| classify(&e)),
            Moderation::KickWith(r) if rights.kick => room.kick_user(&target, reason(Some(r))).await.map_err(|e| classify(&e)),
            Moderation::Ban if rights.ban => room.ban_user(&target, None).await.map_err(|e| classify(&e)),
            Moderation::BanWith(r) if rights.ban => room.ban_user(&target, reason(Some(r))).await.map_err(|e| classify(&e)),
            Moderation::Unban(r) if pl.user_can_unban_user(me, &target) => room.unban_user(&target, reason(r.as_ref())).await.map_err(|e| classify(&e)),
            Moderation::Invite if pl.user_can_invite(me) => room.invite_user_by_id(&target).await.map_err(|e| classify(&e)),
            Moderation::SetPower(l) if rights.levels.contains(l) => room
                .update_power_levels(vec![(&*target, matrix_sdk::ruma::Int::from(*l as i32))])
                .await
                .map(drop)
                .map_err(|e| classify(&e)),
            Moderation::RedactMessages(ids) if redact && me != target => {
                for id in ids {
                    let id = matrix_sdk::ruma::EventId::parse(id).map_err(|_| OperationError::InvalidInput)?;
                    room.redact(&id, None, None).await.map_err(|e| classify(&e.into()))?;
                }
                Ok(())
            }
            _ => Err(OperationError::Forbidden),
        }
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

/// Sets the member's role (power level 100/50/0) with the SDK's `update_power_levels`; rights are re-read
/// at the moment of the call, so hierarchy and creators are enforced here too (`Forbidden`).
#[cfg(feature = "sdk")]
pub(crate) async fn set_member_role(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    user_id: &str,
    role: matrix_domain::role::MemberRole,
) -> Result<(), OperationError> {
    moderate(client, account, scope, user_id, &Moderation::SetPower(role.power_level())).await
}

#[cfg(feature = "sdk")]
#[derive(Default)]
pub(crate) struct OperationJournal {
    /// Request, its confirmed event, and the transaction ID the SDK send queue gave a redaction.
    requests: Vec<(
        OperationRequest,
        Option<EventId>,
        Option<matrix_sdk::ruma::OwnedTransactionId>,
    )>,
}

#[cfg(feature = "sdk")]
pub(crate) async fn perform(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    state: &tokio::sync::Mutex<OperationJournal>,
    request: &OperationRequest,
) -> Result<EventId, OperationError> {
    if request.scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let mut state = state.lock().await;
    let existing = state
        .requests
        .iter()
        .position(|(original, ..)| original.transaction_id == request.transaction_id);
    let mut retry = false;
    let index = if let Some(index) = existing {
        let (original, event, _) = &state.requests[index];
        if original != request {
            return Err(OperationError::InvalidInput);
        }
        if let Some(event) = event {
            return Ok(event.clone());
        }
        retry = true;
        index
    } else {
        if request.transaction_id.as_str().is_empty()
            || matches!(&request.operation, Operation::React { key } if key.trim().is_empty() || key.len() > 32768)
        {
            return Err(OperationError::InvalidInput);
        }
        // shortcut: 128 unconfirmed-or-kept intents per session; replies and edits evict a confirmed
        // one instead of refusing. Use a durable journal (P283a) before lifting this cap.
        if state.requests.len() >= 128 {
            let confirmed = state.requests.iter().position(|(_, e, _)| e.is_some());
            match (&request.operation, confirmed) {
                (Operation::Reply { .. } | Operation::Edit { .. }, Some(old)) => {
                    state.requests.remove(old);
                }
                _ => return Err(OperationError::Unavailable),
            }
        }
        state.requests.push((request.clone(), None, None));
        state.requests.len() - 1
    };
    let mut queued = state.requests[index].2.clone();
    let event = perform_uncached(client, timelines, account, request, retry, &mut queued).await;
    state.requests[index].2 = queued;
    let event = event?;
    state.requests[index].1 = Some(event.clone());
    Ok(event)
}

/// The id of the next event the send queue reports as sent that satisfies `want` (read back from
/// the server: the queue chooses its own transaction ids).
#[cfg(feature = "sdk")]
pub(crate) async fn sent_event(
    updates: &mut tokio::sync::broadcast::Receiver<matrix_sdk::send_queue::SendQueueUpdate>,
    room: &matrix_sdk::Room,
    want: impl Fn(&serde_json::Value) -> bool,
) -> Result<EventId, OperationError> {
    use matrix_sdk::send_queue::RoomSendQueueUpdate::SentEvent;
    use tokio::sync::broadcast::error::RecvError;
    tokio::time::timeout(WAIT, async {
        loop {
            let update = match updates.recv().await {
                Ok(update) => update,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return Err(OperationError::Transport),
            };
            let SentEvent { event_id, .. } = update.update else {
                continue;
            };
            if update.room_id != room.room_id() {
                continue;
            }
            let Ok(event) = room.event(&event_id, None).await else {
                continue;
            };
            let event: serde_json::Value = serde_json::from_str(event.raw().json().get())
                .map_err(|_| OperationError::Transport)?;
            if want(&event) {
                return Ok(EventId::new(event_id.as_str()));
            }
        }
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

#[cfg(feature = "sdk")]
async fn perform_uncached(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    request: &OperationRequest,
    retry: bool,
    queued: &mut Option<matrix_sdk::ruma::OwnedTransactionId>,
) -> Result<EventId, OperationError> {
    use matrix_sdk::{room::edit::EditedContent, ruma::EventId as MatrixEventId};
    let target =
        MatrixEventId::parse(request.target.as_str()).map_err(|_| OperationError::InvalidInput)?;
    if request.transaction_id.as_str().is_empty() {
        return Err(OperationError::InvalidInput);
    }
    let text = match &request.operation {
        Operation::Reply { body } | Operation::Edit { body } => Some(body),
        Operation::React { key } => Some(key),
        Operation::Redact | Operation::EndPoll | Operation::Vote { .. } => None,
    };
    if matches!(&request.operation, Operation::Vote { answers } if answers.len() > 20 || answers.iter().any(|a| a.chars().count() > 340)) {
        return Err(OperationError::InvalidInput);
    }
    if text.is_some_and(|text| text.trim().is_empty() || text.len() > 32768) {
        return Err(OperationError::InvalidInput);
    }
    let user = client.user_id().ok_or(OperationError::Unavailable)?;
    let redact = matches!(request.operation, Operation::Redact);
    let (room, timeline, mut item) =
        locate(client, timelines, account, &request.scope, &target, !redact).await?;
    // A reaction is no Timeline item: look further only for a message.
    if redact && item.is_none() && !is_reaction(&room, &target).await {
        item = locate(client, timelines, account, &request.scope, &target, true)
            .await?
            .2;
        if item.is_none() {
            return Err(OperationError::Unavailable);
        }
    }
    let relation = |e: &serde_json::Value| e["content"]["m.relates_to"].clone();
    match &request.operation {
        Operation::Redact => {
            let Some(item) = item else {
                return redact_reaction(&room, user, request, retry).await;
            };
            // The SDK has no permission check: power levels stay ours.
            let levels = room
                .power_levels()
                .await
                .map_err(|_| OperationError::Unavailable)?;
            let allowed = if item.is_own() {
                levels.user_can_redact_own_event(user)
            } else {
                levels.user_can_redact_event_of_other(user)
            };
            if !allowed {
                return Err(OperationError::Forbidden);
            }
            // An uncertain retry never queues a second redaction: it only looks for the first.
            if !retry {
                use matrix_sdk::send_queue::{
                    LocalEcho, LocalEchoContent, RoomSendQueueUpdate::NewLocalEvent,
                };
                let mut updates = client.send_queue().subscribe();
                timeline
                    .redact(&item.identifier(), None)
                    .await
                    .map_err(|_| OperationError::Transport)?;
                // The SDK picks the transaction ID of the queued redaction; it is the proof later.
                while let Ok(update) = updates.try_recv() {
                    if let NewLocalEvent(LocalEcho {
                        transaction_id,
                        content: LocalEchoContent::Redaction { redacts, .. },
                    }) = update.update
                        && redacts == target
                    {
                        *queued = Some(transaction_id);
                    }
                }
            }
            // Only a retry needs the proof: the first attempt queued this very redaction. A server
            // that omits `unsigned.transaction_id` never confirms a retry (it stays Transport).
            let proof = queued.as_ref().filter(|_| retry).map(|t| t.as_str());
            tokio::time::timeout(WAIT, async {
                loop {
                    if let Ok(id) = redaction_of(&room, user, &target, proof).await {
                        return id;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
            })
            .await
            .map_err(|_| OperationError::Transport)
        }
        Operation::React { key } => {
            let item = item.ok_or(OperationError::Unavailable)?;
            let reacted = item
                .content()
                .reactions()
                .is_some_and(|r| r.get(key.as_str()).is_some_and(|by| by.contains_key(user)));
            if retry {
                // The SDK shows no event ID for a reaction it already confirmed.
                if let Ok(id) = own_reaction(&room, user, &target, key).await {
                    return Ok(id);
                }
            } else if reacted {
                return Err(OperationError::InvalidInput);
            }
            let mut updates = client.send_queue().subscribe();
            let id = item.identifier();
            let added = timeline
                .toggle_reaction(&id, key)
                .await
                .map_err(|_| OperationError::Transport)?;
            // `false`: it removed a queued echo of the first attempt; add it again.
            if !added {
                timeline
                    .toggle_reaction(&id, key)
                    .await
                    .map_err(|_| OperationError::Transport)?;
            }
            sent_event(&mut updates, &room, |e| {
                let r = relation(e);
                e["type"] == "m.reaction"
                    && r["rel_type"] == "m.annotation"
                    && r["event_id"].as_str() == Some(target.as_str())
                    && r["key"].as_str() == Some(key.as_str())
            })
            .await
        }
        Operation::Edit { body } => {
            let item = item.ok_or(OperationError::Unavailable)?;
            if item.content().as_message().is_none() {
                return Err(OperationError::Unavailable);
            }
            if !item.is_own() {
                return Err(OperationError::Forbidden);
            }
            if !item.is_editable() {
                return Err(OperationError::Unavailable);
            }
            let mut updates = client.send_queue().subscribe();
            let content = crate::sending::markdown_content(body, &[]);
            timeline
                .edit(
                    &item.identifier(),
                    EditedContent::RoomMessage(content.into()),
                )
                .await
                .map_err(|_| OperationError::Transport)?;
            sent_event(&mut updates, &room, |e| {
                let r = relation(e);
                r["rel_type"] == "m.replace" && r["event_id"].as_str() == Some(target.as_str())
            })
            .await
        }
        Operation::Vote { .. } | Operation::EndPoll => {
            use matrix_sdk::ruma::events::poll::{unstable_end::UnstablePollEndEventContent, unstable_response::UnstablePollResponseEventContent};
            let item = item.ok_or(OperationError::Unavailable)?;
            if item.content().as_poll().is_none() {
                return Err(OperationError::Unavailable);
            }
            let ending = matches!(request.operation, Operation::EndPoll);
            if ending && !item.is_own() {
                return Err(OperationError::Forbidden);
            }
            let mut updates = client.send_queue().subscribe();
            let kind = if let Operation::Vote { answers } = &request.operation {
                timeline
                    .send(UnstablePollResponseEventContent::new(answers.clone(), target.clone()).into())
                    .await
            } else {
                timeline
                    .send(UnstablePollEndEventContent::new("Sondaggio chiuso", target.clone()).into())
                    .await
            };
            kind.map_err(|_| OperationError::Transport)?;
            let wanted = if ending { "org.matrix.msc3381.poll.end" } else { "org.matrix.msc3381.poll.response" };
            sent_event(&mut updates, &room, |e| {
                e["type"] == wanted && relation(e)["event_id"].as_str() == Some(target.as_str())
            })
            .await
        }
        Operation::Reply { body } => {
            if item.is_none() {
                return Err(OperationError::Unavailable);
            }
            let mut updates = client.send_queue().subscribe();
            let content = crate::sending::markdown_content(body, &[]);
            timeline
                .send_reply(content.into(), target.clone())
                .await
                .map_err(|_| OperationError::Transport)?;
            sent_event(&mut updates, &room, |e| {
                relation(e)["m.in_reply_to"]["event_id"].as_str() == Some(target.as_str())
            })
            .await
        }
    }
}

#[cfg(feature = "sdk")]
async fn is_reaction(room: &matrix_sdk::Room, id: &matrix_sdk::ruma::EventId) -> bool {
    let Ok(Ok(event)) = tokio::time::timeout(WAIT, room.event(id, None)).await else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(event.raw().json().get())
        .is_ok_and(|event| event["type"] == "m.reaction")
}

/// A reaction is not a Timeline item, so the SDK cannot address it by event ID: removing one by
/// its ID stays a plain redaction with our transaction ID, recoverable with that ID as proof.
#[cfg(feature = "sdk")]
async fn redact_reaction(
    room: &matrix_sdk::Room,
    user: &matrix_sdk::ruma::UserId,
    request: &OperationRequest,
    retry: bool,
) -> Result<EventId, OperationError> {
    let target = matrix_sdk::ruma::EventId::parse(request.target.as_str())
        .map_err(|_| OperationError::InvalidInput)?;
    if retry {
        return redaction_of(room, user, &target, Some(request.transaction_id.as_str())).await;
    }
    let event = tokio::time::timeout(WAIT, room.event(&target, None))
        .await
        .map_err(|_| OperationError::Transport)?
        .map_err(|_| OperationError::Unavailable)?;
    let event: serde_json::Value =
        serde_json::from_str(event.raw().json().get()).map_err(|_| OperationError::Unavailable)?;
    if event["type"] != "m.reaction" {
        return Err(OperationError::Unavailable);
    }
    let response = tokio::time::timeout(
        WAIT,
        room.redact(&target, None, Some(request.transaction_id.as_str().into())),
    )
    .await
    .map_err(|_| OperationError::Transport)?
    .map_err(|error| {
        if matches!(
            error.client_api_error_kind(),
            Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
        ) {
            OperationError::Forbidden
        } else {
            OperationError::Transport
        }
    })?;
    Ok(EventId::new(response.event_id.as_str()))
}

/// The redaction of `target` as stored by the server, if this user made it. With `transaction`,
/// only the one sent with that transaction ID counts. A deleted row alone is no proof.
#[cfg(feature = "sdk")]
async fn redaction_of(
    room: &matrix_sdk::Room,
    user: &matrix_sdk::ruma::UserId,
    target: &matrix_sdk::ruma::EventId,
    transaction: Option<&str>,
) -> Result<EventId, OperationError> {
    use matrix_sdk::ruma::EventId as MatrixEventId;
    tokio::time::timeout(WAIT, async {
        let original = room
            .event(target, None)
            .await
            .map_err(|_| OperationError::Transport)?;
        let original: serde_json::Value = serde_json::from_str(original.raw().json().get())
            .map_err(|_| OperationError::Transport)?;
        let id = original["unsigned"]["redacted_because"]["event_id"]
            .as_str()
            .ok_or(OperationError::Transport)?;
        let id = MatrixEventId::parse(id).map_err(|_| OperationError::Transport)?;
        let event = room
            .event(&id, None)
            .await
            .map_err(|_| OperationError::Transport)?;
        let event: serde_json::Value = serde_json::from_str(event.raw().json().get())
            .map_err(|_| OperationError::Transport)?;
        let redacts = event["content"].get("redacts").unwrap_or(&event["redacts"]);
        if original["event_id"].as_str() != Some(target.as_str())
            || event["event_id"].as_str() != Some(id.as_str())
            || event["type"] != "m.room.redaction"
            || event["sender"].as_str() != Some(user.as_str())
            || event["room_id"]
                .as_str()
                .is_some_and(|id| id != room.room_id().as_str())
            || redacts.as_str() != Some(target.as_str())
            || transaction.is_some_and(|t| event["unsigned"]["transaction_id"].as_str() != Some(t))
        {
            return Err(OperationError::Transport);
        }
        Ok(EventId::new(id.as_str()))
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

/// The user's own reaction `key` on `target` as stored by the server (the SDK shows no event ID for
/// one it already confirmed).
#[cfg(feature = "sdk")]
async fn own_reaction(
    room: &matrix_sdk::Room,
    user: &matrix_sdk::ruma::UserId,
    target: &matrix_sdk::ruma::EventId,
    key: &str,
) -> Result<EventId, OperationError> {
    use matrix_sdk::{
        room::{IncludeRelations, RelationsOptions},
        ruma::events::relation::RelationType,
    };
    tokio::time::timeout(WAIT, async {
        let mut from = None;
        // shortcut: 20 pages of 25 annotations, then the reaction counts as not found.
        for _ in 0..20 {
            let page = room
                .relations(
                    target.to_owned(),
                    RelationsOptions {
                        from,
                        limit: Some(25u32.into()),
                        include_relations: IncludeRelations::RelationsOfType(
                            RelationType::Annotation,
                        ),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|_| OperationError::Transport)?;
            for event in page.chunk {
                let value: serde_json::Value = serde_json::from_str(event.raw().json().get())
                    .map_err(|_| OperationError::Transport)?;
                if value["sender"].as_str() == Some(user.as_str())
                    && value["type"] == "m.reaction"
                    && value["content"]["m.relates_to"]["key"].as_str() == Some(key)
                    && let Some(id) = value["event_id"].as_str()
                {
                    return Ok(EventId::new(id));
                }
            }
            from = page.next_batch_token;
            if from.is_none() {
                break;
            }
        }
        Err(OperationError::Transport)
    })
    .await
    .map_err(|_| OperationError::Transport)?
}
#[cfg(all(test, feature = "sdk"))]
mod tests {

    #[cfg(feature = "sdk")]
    fn api_error(status: u16, kind: Option<matrix_sdk::ruma::api::error::ErrorKind>) -> matrix_sdk::Error {
        use matrix_sdk::ruma::api::error::{Error, ErrorBody, FromHttpResponseError, StandardErrorBody};
        let body = match kind {
            Some(kind) => ErrorBody::Standard(StandardErrorBody::new(kind, "no".into())),
            None => ErrorBody::Json(serde_json::json!({})),
        };
        let status = matrix_sdk::reqwest::StatusCode::from_u16(status).unwrap();
        let error = Error::new(status, body);
        matrix_sdk::Error::Http(Box::new(matrix_sdk::HttpError::Api(Box::new(FromHttpResponseError::Server(matrix_sdk::ruma::api::client::uiaa::UiaaResponse::MatrixError(error))))))
    }
    #[cfg(feature = "sdk")]
    #[test]
    fn space_errors_keep_forbidden_apart_from_the_network() {
        use matrix_sdk_ui::spaces::Error as E;
        let forbidden = api_error(403, Some(ErrorKind::Forbidden));
        assert_eq!(classify_space(&E::UpdateRelationship(forbidden)), OperationError::Forbidden);
        assert_eq!(classify_space(&E::UpdateRelationship(api_error(502, None))), OperationError::Transport);
        assert_eq!(classify_space(&E::RoomNotFound(matrix_sdk::ruma::owned_room_id!("!a:b.c"))), OperationError::Unavailable);
        assert_eq!(classify_space(&E::UserIdNotFound), OperationError::Transport);
        assert_eq!(space_error_line(OperationError::Forbidden), "Non consentito dal server");
        assert!(!space_error_line(OperationError::Transport).contains('\n'));
    }
    #[cfg(feature = "sdk")]
    #[tokio::test(start_paused = true)]
    async fn space_write_times_out_and_tolerates_a_missing_parent_link() {
        use matrix_sdk_ui::spaces::Error as E;
        let slow = space_write(std::future::pending());
        assert_eq!(slow.await, Err(OperationError::Transport));
        assert_eq!(space_write(async { Err(E::UpdateInverseRelationship(api_error(403, Some(ErrorKind::Forbidden)))) }).await, Ok(()));
        assert_eq!(
            space_write(async { Err(E::UpdateRelationship(api_error(403, Some(ErrorKind::Forbidden)))) }).await,
            Err(OperationError::Forbidden)
        );
    }

    #[cfg(feature = "sdk")]
    #[test]
    fn blank_reason_counts_as_none() {
        assert_eq!(reason(None), None);
        assert_eq!(reason(Some(&"  ".to_string())), None);
        assert_eq!(reason(Some(&" spam ".to_string())), Some("spam"));
    }

    #[test]
    fn space_request_is_an_unencrypted_private_m_space() {
        use matrix_sdk::ruma::{api::client::room::{Visibility, create_room::v3::RoomPreset}, room::RoomType};
        for (public, preset) in [(false, RoomPreset::PrivateChat), (true, RoomPreset::PublicChat)] {
            let r = space_request("Casa", Some(" tema "), public);
            assert_eq!(r.creation_content.unwrap().deserialize().unwrap().room_type, Some(RoomType::Space));
            assert_eq!(r.preset, Some(preset));
            // Public is only requested after creation.
            assert_eq!(r.visibility, Visibility::Private);
            assert!(r.initial_state.is_empty());
            assert_eq!(r.topic.as_deref(), Some("tema"));
        }
    }

    #[test]
    fn partial_space_outcomes_are_distinct_one_line_notices() {
        assert_eq!(SpaceNotice::NotPublished.line(), "Spazio creato, non pubblicato nella directory");
        assert_ne!(SpaceNotice::AvatarFailed(OperationError::Transport), SpaceNotice::NotPublished);
        assert!(!SpaceNotice::AvatarFailed(OperationError::Forbidden).line().contains('\n'));
    }
    #[cfg(feature = "sdk")]
    #[test]
    fn only_a_join_content_counts_as_still_joined() {
        use super::content_is_join;
        assert!(content_is_join(r#"{"membership":"join"}"#));
        assert!(!content_is_join(r#"{"membership":"leave"}"#));
        assert!(!content_is_join("not json"));
    }

    use super::*;

    fn levels(users: &[(&str, i32)]) -> matrix_sdk::ruma::events::room::power_levels::RoomPowerLevels {
        use matrix_sdk::ruma::{Int, events::room::power_levels::*, room_version_rules::AuthorizationRules};
        let rules = AuthorizationRules::V10;
        let mut c = RoomPowerLevelsEventContent::new(&rules);
        for (u, l) in users {
            c.users.insert(matrix_sdk::ruma::UserId::parse(*u).unwrap(), Int::from(*l));
        }
        RoomPowerLevels::new(c.into(), &rules, [])
    }

    #[test]
    fn moderator_rights_stop_at_own_level_and_at_peers() {
        let id = |s| matrix_sdk::ruma::UserId::parse(s).unwrap();
        let pl = levels(&[("@mod:x", 50), ("@mod2:x", 50), ("@admin:x", 100)]);
        let rights = |me, who, joined| member_rights(&pl, &id(me), &id(who), joined);
        // a plain user may do nothing, and deletes nothing
        assert_eq!(rights("@u:x", "@v:x", true), (MemberRights::default(), false));
        // the moderator: kick/ban a plain member, offers Moderatore and no Utente (already 0), nothing above self
        let (r, redact) = rights("@mod:x", "@v:x", true);
        assert!(r.kick && r.ban && redact);
        assert_eq!(r.levels, vec![50]);
        // not over a peer or a superior, not over self, not over a non-member
        for who in ["@mod2:x", "@admin:x", "@mod:x"] {
            let (r, _) = rights("@mod:x", who, true);
            assert!(!r.kick && !r.ban, "{who}");
            assert!(r.levels.is_empty() || who == "@mod:x", "{who}");
        }
        assert_eq!(rights("@mod:x", "@v:x", false).0, MemberRights::default());
        // the admin may give every level the member does not have yet
        assert_eq!(rights("@admin:x", "@mod:x", true).0.levels, vec![0, 100]);
        // role and can_change_role for the UI: custom levels map to the nearest role below, peers/superiors are locked
        use matrix_domain::role::MemberRole::*;
        let by = |me, who| { let r = rights(me, who, true).0; (r.role, r.creator, r.can_change_role) };
        assert_eq!(by("@mod:x", "@v:x"), (Member, false, true));
        assert_eq!(by("@mod:x", "@mod2:x"), (Moderator, false, false));
        assert_eq!(by("@mod:x", "@admin:x"), (Admin, false, false));
        assert_eq!(by("@u:x", "@v:x"), (Member, false, false));
        assert_eq!(by("@admin:x", "@mod:x"), (Moderator, false, true));
        assert_eq!(rights("@admin:x", "@admin:x", true).0.role, Admin);
    }
    #[test]
    fn v12_creator_is_locked_and_admin() {
        use matrix_sdk::ruma::{UserId, events::room::power_levels::*, room_version_rules::AuthorizationRules};
        let id = |s| UserId::parse(s).unwrap();
        let rules = AuthorizationRules::V12;
        let c = RoomPowerLevelsEventContent::new(&rules);
        let pl = RoomPowerLevels::new(c.into(), &rules, [id("@c:x")]);
        let (r, _) = member_rights(&pl, &id("@c:x"), &id("@o:x"), true);
        assert!(!r.creator && r.can_change_role && r.levels == vec![50, 100]);
        let (r, _) = member_rights(&pl, &id("@o:x"), &id("@c:x"), true);
        assert!(r.creator && !r.can_change_role && r.role == matrix_domain::role::MemberRole::Admin);
    }
    #[test]
    fn edit_rights_follow_the_state_event_levels() {
        use matrix_sdk::ruma::{Int, UserId, events::{TimelineEventType, room::power_levels::*}, room_version_rules::AuthorizationRules};
        let rules = AuthorizationRules::V10;
        let mut c = RoomPowerLevelsEventContent::new(&rules);
        c.users.insert(UserId::parse("@mod:x").unwrap(), Int::from(50));
        c.events.insert(TimelineEventType::RoomTopic, Int::from(0));
        c.events.insert(TimelineEventType::RoomAvatar, Int::from(100));
        let pl = RoomPowerLevels::new(c.into(), &rules, []);
        let rights = |u| crate::EditRights::from_levels(&pl, &UserId::parse(u).unwrap());
        // plain user: only what the room opened to level 0
        assert_eq!(rights("@u:x"), crate::EditRights { name: false, topic: true, avatar: false });
        // moderator (50, default for name): name and topic, not the avatar set to 100
        assert_eq!(rights("@mod:x"), crate::EditRights { name: true, topic: true, avatar: false });
        assert!(!crate::EditRights::default().any());
    }
    #[test]
    fn edit_rights_by_role_with_default_levels() {
        let id = |s| matrix_sdk::ruma::UserId::parse(s).unwrap();
        let pl = levels(&[("@admin:x", 100), ("@mod:x", 50)]);
        let rights = |u| crate::EditRights::from_levels(&pl, &id(u));
        let all = crate::EditRights { name: true, topic: true, avatar: true };
        assert_eq!(rights("@admin:x"), all);
        // default room: name/topic/avatar need 50, so the moderator qualifies
        assert_eq!(rights("@mod:x"), all);
        // plain member (0): nothing, so «Modifica» stays hidden
        assert!(!rights("@u:x").any());
    }

    #[test]
    fn edit_rights_follow_promotion_and_demotion() {
        let id = matrix_sdk::ruma::UserId::parse("@u:x").unwrap();
        let rights = |pl| crate::EditRights::from_levels(&pl, &id);
        assert!(!rights(levels(&[])).any());
        assert!(rights(levels(&[("@u:x", 50)])).any(), "promoted");
        assert!(!rights(levels(&[("@u:x", 49)])).any(), "just below the threshold");
        assert!(!rights(levels(&[("@u:x", 0)])).any(), "demoted again");
    }

    #[test]
    fn edit_rights_can_be_partial() {
        use matrix_sdk::ruma::{Int, UserId, events::{TimelineEventType, room::power_levels::*}, room_version_rules::AuthorizationRules};
        let rules = AuthorizationRules::V10;
        let mut c = RoomPowerLevelsEventContent::new(&rules);
        c.users.insert(UserId::parse("@mod:x").unwrap(), Int::from(50));
        c.events.insert(TimelineEventType::RoomName, Int::from(100));
        c.events.insert(TimelineEventType::RoomTopic, Int::from(100));
        let pl = RoomPowerLevels::new(c.into(), &rules, []);
        let r = crate::EditRights::from_levels(&pl, &UserId::parse("@mod:x").unwrap());
        assert_eq!(r, crate::EditRights { name: false, topic: false, avatar: true });
        assert!(r.any(), "one field is enough to keep the entry");
    }

    #[test]
    fn full_reaction_journal_keeps_confirmed_identity_without_accepting_new_writes() {
        use matrix_domain::{AccountId, RoomId};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let client = matrix_sdk::Client::builder()
                .homeserver_url("https://example.invalid")
                .server_versions(vec![matrix_sdk::ruma::api::MatrixVersion::V1_0])
                .build()
                .await
                .unwrap();
            let request = OperationRequest {
                scope: RoomScope {
                    account_id: AccountId::new(52),
                    room_id: RoomId::new("!room:example.invalid"),
                },
                transaction_id: TransactionId::new("0"),
                target: EventId::new("$target"),
                operation: Operation::React { key: "👍".into() },
            };
            let state = tokio::sync::Mutex::new(OperationJournal {
                requests: (0..128)
                    .map(|n| {
                        (
                            OperationRequest {
                                transaction_id: TransactionId::new(n.to_string()),
                                ..request.clone()
                            },
                            Some(EventId::new("$confirmed")),
                            None,
                        )
                    })
                    .collect(),
            });
            assert_eq!(
                perform(&client, &crate::room_sync::Timelines::default(), AccountId::new(52), &state, &request).await,
                Ok(EventId::new("$confirmed"))
            );
            let new = OperationRequest {
                transaction_id: TransactionId::new("new"),
                ..request
            };
            assert_eq!(
                perform(&client, &crate::room_sync::Timelines::default(), AccountId::new(52), &state, &new).await,
                Err(OperationError::Unavailable)
            );
            assert_eq!(state.lock().await.requests.len(), 128);
        });
    }
}

#[cfg(all(test, feature = "sdk"))]
#[path = "operations_directory_tests.rs"]
mod directory_tests;
