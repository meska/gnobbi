//! Quick switcher core: local search over rooms/DMs already known and the per-account lists
//! «Visti di recente» / «Ricerche recenti». No UI, no network. Stored locally in the config dir
//! (`quick-search.json`, like `rail-layout.json`), never sent to the server.
use crate::AppDirs;
use matrix_engine::operations::{DirectoryKind, DirectoryRoom};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use unicode_normalization::UnicodeNormalization;

pub const SEEN_LIMIT: usize = 10;
pub const SEARCHED_LIMIT: usize = 8;
const FILE: &str = "quick-search.json";

/// Row shown by the UI. `avatar` is the mxc reference for the `Avatars` cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    /// Bridge network when known, else the Matrix ID.
    pub secondary: String,
}

/// «Cerca» entries (order as Element): each one becomes a chip in the field and limits the
/// results to its kind. The two directory kinds also show the server selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Spaces,
    Rooms,
    People,
}

impl Category {
    pub const ALL: [Category; 3] = [Category::Spaces, Category::Rooms, Category::People];

    pub fn label(self) -> &'static str {
        match self {
            Category::Spaces => crate::i18n::t("Public spaces"),
            Category::Rooms => crate::i18n::t("Public rooms"),
            Category::People => crate::i18n::t("People"),
        }
    }

    /// Screen-reader name of the chip.
    pub fn chip_label(self) -> String {
        crate::i18n::t("Filter: {label}").replace("{label}", self.label())
    }

    /// What the public directory is asked for; `None` for people.
    pub fn kind(self) -> Option<DirectoryKind> {
        match self {
            Category::Spaces => Some(DirectoryKind::Spaces),
            Category::Rooms => Some(DirectoryKind::Rooms),
            Category::People => None,
        }
    }
}

/// `(term, server, kind)` of a directory search.
pub type DirectoryRequest = (String, Option<String>, Option<DirectoryKind>);

/// Public rooms shown under the local results when no chip is active.
pub const ROOMS_LIMIT: usize = 5;

/// The directory request the overlay wants now: `(term, server, kind)`, or `None` for no request
/// at all. A directory chip asks at once (empty term = «Suggerimenti», one page of `server`); no
/// chip asks only for a typed term, always the active account's server; people ask nothing here.
pub fn room_request(
    chip: Option<Category>,
    term: &str,
    server: Option<String>,
) -> Option<DirectoryRequest> {
    let term = term.trim();
    let typed = term.chars().count() >= matrix_engine::search::MIN_CHARS;
    let term = if typed { term.to_owned() } else { String::new() };
    match chip {
        Some(c) => c.kind().map(|k| (term, server, Some(k))),
        None => typed.then_some((term, None, Some(DirectoryKind::Rooms))),
    }
}

/// Directory rows as overlay rows: the shared list's detail line (alias, members) is the secondary.
pub fn room_hits<'a>(rooms: impl IntoIterator<Item = &'a DirectoryRoom>, limit: usize) -> Vec<Hit> {
    use crate::search_list::Hit as _;
    rooms
        .into_iter()
        .take(limit)
        .map(|r| Hit { id: r.room_id.clone(), name: r.name.clone(), avatar: r.avatar_url.clone(), secondary: r.detail() })
        .collect()
}

/// Server users shown under the local results when no chip is active.
pub const USERS_LIMIT: usize = 5;

/// Esc: first press removes the chip, the next closes the overlay.
pub fn escape_closes(chip: Option<Category>) -> bool {
    chip.is_none()
}

/// Backspace on an empty field removes the chip.
pub fn backspace_removes_chip(chip: Option<Category>, term: &str) -> bool {
    chip.is_some() && term.is_empty()
}

/// The term to send to the user search, if it differs from what was last sent. Nothing goes out
/// for an empty field that never searched (chip or not); clearing a sent term cancels it.
pub fn user_request(term: &str, sent: Option<&str>) -> Option<String> {
    (term != sent.unwrap_or("")).then(|| term.to_owned())
}

/// «Utenti» rows: capped at `USERS_LIMIT` without a chip, all of them with the chip.
pub fn user_hits(entries: Vec<crate::new_chat::Entry>, chip: Option<Category>) -> Vec<Hit> {
    let limit = if chip.is_some() { usize::MAX } else { USERS_LIMIT };
    entries
        .into_iter()
        .take(limit)
        .map(|e| Hit {
            name: e.name.filter(|n| !n.is_empty()).unwrap_or_else(|| e.user_id.clone()),
            id: e.user_id.clone(),
            avatar: e.avatar,
            secondary: e.user_id,
        })
        .collect()
}

/// A room or DM known locally.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    /// Room ID.
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    /// DM: the peer's Matrix user ID and display name.
    pub user_id: Option<String>,
    pub contact: Option<String>,
    pub avatar: Option<String>,
    pub bridge: Option<String>,
    /// Last activity (ms); higher sorts first among equal matches.
    pub activity: u64,
}

impl Entry {
    /// Entry for a sidebar row. Servers (Tuwunel) send empty strings where Synapse sends
    /// nothing: those count as absent. The DM contact name is the room name (SDK display name).
    pub fn from_room(room: &crate::sidebar::SidebarRoom, activity: u64) -> Self {
        let present = |s: &str| !s.is_empty();
        let user_id = room.info.dm_user_id.clone().filter(|u| present(u));
        Entry {
            id: room.scope.room_id.as_str().to_owned(),
            name: room.name.clone(),
            aliases: room.info.aliases.iter().filter(|a| present(a)).cloned().collect(),
            contact: user_id.as_ref().map(|_| room.name.clone()),
            user_id,
            avatar: room.info.avatar.clone().filter(|a| present(a)),
            activity,
            ..Default::default()
        }
    }
}

/// Lowercase, accents stripped (NFD without combining marks).
pub fn normalize(s: &str) -> String {
    s.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// 0 exact, 1 prefix, 2 word start, 3 substring.
fn rank(field: &str, q: &str) -> Option<u8> {
    if field == q {
        return Some(0);
    }
    if field.starts_with(q) {
        return Some(1);
    }
    let mut best = None;
    for (i, _) in field.match_indices(q) {
        if field[..i]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_alphanumeric())
        {
            return Some(2);
        }
        best = Some(3);
    }
    best
}

struct Prepared {
    entry: Entry,
    fields: Vec<String>,
}

#[derive(Default)]
pub struct Index(Vec<Prepared>);

impl Index {
    pub fn new(entries: impl IntoIterator<Item = Entry>) -> Self {
        let mut index = Self::default();
        for e in entries {
            index.upsert(e);
        }
        index
    }

    /// Adds or replaces the entry with the same id (incremental update).
    pub fn upsert(&mut self, entry: Entry) {
        let fields = std::iter::once(&entry.name)
            .chain(&entry.aliases)
            .chain(&entry.contact)
            .chain(&entry.user_id)
            .chain(std::iter::once(&entry.id))
            .map(|f| normalize(f))
            .collect();
        let prepared = Prepared { entry, fields };
        match self.0.iter_mut().find(|p| p.entry.id == prepared.entry.id) {
            Some(slot) => *slot = prepared,
            None => self.0.push(prepared),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.0.retain(|p| p.entry.id != id);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Index {
    /// Best field match decides relevance; ties: most recent activity, then name, then id.
    /// An empty query yields nothing (the UI shows the recent lists instead).
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let q = normalize(query.trim());
        if q.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(u8, &Entry)> = self
            .0
            .iter()
            .filter_map(|p| {
                let r = p.fields.iter().filter_map(|f| rank(f, &q)).min()?;
                Some((r, &p.entry))
            })
            .collect();
        found.sort_by(|(ra, a), (rb, b)| {
            ra.cmp(rb)
                .then(b.activity.cmp(&a.activity))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });
        found
            .into_iter()
            .take(limit)
            .map(|(_, e)| Hit {
                id: e.id.clone(),
                name: e.name.clone(),
                avatar: e.avatar.clone(),
                secondary: e
                    .bridge
                    .clone()
                    .unwrap_or_else(|| e.user_id.clone().unwrap_or_else(|| e.id.clone())),
            })
            .collect()
    }
}

/// What a recent list remembers: enough to draw the row without the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
}

impl From<&Hit> for Recent {
    fn from(h: &Hit) -> Self {
        Recent {
            id: h.id.clone(),
            name: h.name.clone(),
            avatar: h.avatar.clone(),
        }
    }
}

/// One account's lists, most recent first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recents {
    seen: Vec<Recent>,
    searched: Vec<Recent>,
}

fn push(list: &mut Vec<Recent>, item: Recent, limit: usize) {
    list.retain(|r| r.id != item.id);
    list.insert(0, item);
    list.truncate(limit);
}

impl Recents {
    /// A room was opened.
    pub fn opened(&mut self, item: Recent) {
        push(&mut self.seen, item, SEEN_LIMIT);
    }
    /// A room or user was picked from the search results.
    pub fn picked(&mut self, item: Recent) {
        push(&mut self.searched, item, SEARCHED_LIMIT);
    }
    pub fn seen(&self) -> &[Recent] {
        &self.seen
    }
    pub fn searched(&self) -> &[Recent] {
        &self.searched
    }
    /// The room is gone (left): it leaves both lists.
    pub fn forget(&mut self, id: &str) {
        self.seen.retain(|r| r.id != id);
        self.searched.retain(|r| r.id != id);
    }
    /// «Svuota» on «Ricerche recenti»; «Visti di recente» is history and stays.
    pub fn clear_searched(&mut self) {
        self.searched.clear();
    }
}

fn parse_list(v: Option<&Value>, limit: usize) -> Vec<Recent> {
    let mut out: Vec<Recent> = Vec::new();
    for item in v.and_then(Value::as_array).into_iter().flatten() {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        if out.iter().any(|r| r.id == id) {
            continue;
        }
        out.push(Recent {
            id: id.into(),
            name: item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .into(),
            avatar: item.get("avatar").and_then(Value::as_str).map(Into::into),
        });
    }
    out.truncate(limit);
    out
}

fn to_json(list: &[Recent]) -> Value {
    list.iter()
        .map(|r| json!({ "id": r.id, "name": r.name, "avatar": r.avatar }))
        .collect()
}

/// Recents of all accounts, keyed by catalog account id. Callers touch only the active one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentsStore(BTreeMap<u64, Recents>);

impl RecentsStore {
    /// Missing or corrupt file: empty lists. A bad entry is skipped.
    pub fn load(dirs: &AppDirs) -> Self {
        let Ok(bytes) = std::fs::read(dirs.config.join(FILE)) else {
            return Self::default();
        };
        let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&bytes) else {
            return Self::default();
        };
        Self(
            map.iter()
                .filter_map(|(key, v)| {
                    Some((
                        key.parse().ok()?,
                        Recents {
                            seen: parse_list(v.get("seen"), SEEN_LIMIT),
                            searched: parse_list(v.get("searched"), SEARCHED_LIMIT),
                        },
                    ))
                })
                .collect(),
        )
    }

    pub fn get(&self, account: u64) -> Recents {
        self.0.get(&account).cloned().unwrap_or_default()
    }

    /// The account is gone: its names, ids and avatars go with it.
    pub fn remove(&mut self, account: u64) {
        self.0.remove(&account);
    }

    pub fn set(&mut self, account: u64, recents: Recents) {
        self.0.insert(account, recents);
    }

    /// Atomic write (temp file then rename), owner-only.
    pub fn save(&self, dirs: &AppDirs) -> std::io::Result<()> {
        #[cfg(unix)]
        crate::create_private_dir(&dirs.config)?;
        #[cfg(not(unix))]
        std::fs::create_dir_all(&dirs.config)?;
        let body: Map<String, Value> = self
            .0
            .iter()
            .map(|(id, r)| {
                (
                    id.to_string(),
                    json!({ "seen": to_json(&r.seen), "searched": to_json(&r.searched) }),
                )
            })
            .collect();
        let tmp = dirs.config.join(format!("{FILE}.tmp"));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        opts.open(&tmp)?
            .write_all(Value::Object(body).to_string().as_bytes())?;
        std::fs::rename(tmp, dirs.config.join(FILE))
    }
}
