//! «Unisciti»: how a public-directory room reads in the shared result list.
use crate::search_list::Hit;
use matrix_engine::operations::DirectoryRoom;

fn members(n: u64) -> String {
    crate::i18n::plural(n, "{n} member", "{n} members")
}

impl Hit for DirectoryRoom {
    fn key(&self) -> &str {
        &self.room_id
    }
    fn title(&self) -> &str {
        &self.name
    }
    /// «alias · N membri», or just the members when the room has no alias.
    fn detail(&self) -> String {
        match &self.alias {
            Some(alias) => format!("{alias} · {}", members(self.members)),
            None => members(self.members),
        }
    }
    fn avatar(&self) -> Option<&str> {
        self.avatar_url.as_deref()
    }
    /// «name, alias, N membri».
    fn label(&self) -> String {
        match &self.alias {
            Some(alias) => format!("{}, {alias}, {}", self.name, members(self.members)),
            None => format!("{}, {}", self.name, members(self.members)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(alias: Option<&str>, n: u64) -> DirectoryRoom {
        DirectoryRoom {
            room_id: "!r:a.org".into(),
            alias: alias.map(Into::into),
            name: "Rust".into(),
            topic: None,
            members: n,
            avatar_url: None,
            world_readable: false,
            guest_can_join: false,
            join_rule: "public".into(),
            origin: "a.org".into(),
        }
    }

    #[test]
    fn detail_and_label_carry_name_alias_and_members() {
        let r = room(Some("#rust:a.org"), 42);
        assert_eq!((r.detail(), r.label()), ("#rust:a.org · 42 members".into(), "Rust, #rust:a.org, 42 members".into()));
        let r = room(None, 1);
        assert_eq!((r.detail(), r.label()), ("1 member".into(), "Rust, 1 member".into()));
        assert_eq!(r.key(), "!r:a.org");
    }
}
