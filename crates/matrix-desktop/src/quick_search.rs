//! Quick switcher core: local search over rooms/DMs already known and the per-account lists
//! «Visti di recente» / «Ricerche recenti». No UI, no network. Stored locally in the config dir
//! (`quick-search.json`, like `rail-layout.json`), never sent to the server.
use crate::AppDirs;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use unicode_normalization::UnicodeNormalization;

pub const SEEN_LIMIT: usize = 10;
pub const SEARCHED_LIMIT: usize = 8;
const FILE: &str = "quick-search.json";

/// Result groups. Only `Local` is filled here; the others are for server-backed sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Local,
    Users,
    PublicRooms,
}

/// Row shown by the UI. `avatar` is the mxc reference for the `Avatars` cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    /// Bridge network when known, else the Matrix ID.
    pub secondary: String,
}

/// A source of hits for one section. The local `Index` is the only implementor for now;
/// «Utenti» and «Stanze pubbliche» (server directory) will implement it too.
pub trait Source {
    fn section(&self) -> Section;
    fn search(&self, query: &str, limit: usize) -> Vec<Hit>;
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

impl Source for Index {
    fn section(&self) -> Section {
        Section::Local
    }

    /// Best field match decides relevance; ties: most recent activity, then name, then id.
    /// An empty query yields nothing (the UI shows the recent lists instead).
    fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
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
