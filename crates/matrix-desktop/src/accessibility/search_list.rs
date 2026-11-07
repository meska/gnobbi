//! Accessibility nodes of a [`SearchList`](crate::search_list::SearchList), shared by every dialog
//! that has a search field: a `ListBox` «Risultati» (only when there are rows), one
//! `ListBoxOption` per row (label = `Hit::label`, so name, alias and members are read), the
//! «Altri risultati» row as the last option, and a polite live `Status` line. The dialog adds the
//! returned nodes as children, points its `TextInput` at `active` (`set_active_descendant`) and
//! `list` (`set_controls`), and turns a click on `index_of(node)` into its own action.
use crate::search_list::{Hit, Row, Status, more_text};
use accesskit::{Action, Live, Node, NodeId, Role};

/// Ids the dialog reserves: result `i` is node `option + i`.
#[derive(Clone, Copy)]
pub struct Ids {
    pub list: NodeId,
    pub status: NodeId,
    pub option: u64,
}

pub struct Projection {
    /// ListBox then its options; empty when there are no rows (no empty listbox is announced).
    pub list: Vec<(NodeId, Node)>,
    pub status: (NodeId, Node),
    /// The option of the active row.
    pub active: Option<NodeId>,
}

pub fn project<H: Hit>(
    ids: Ids,
    rows: &[Row<'_, H>],
    active: usize,
    status: Option<Status>,
) -> Projection {
    let mut list = Vec::new();
    let mut options = Vec::new();
    let hits = rows.iter().filter(|r| matches!(r, Row::Hit(_))).count();
    if !rows.is_empty() {
        let mut node = Node::new(Role::ListBox);
        node.set_label("Risultati");
        for (i, row) in rows.iter().enumerate() {
            let id = NodeId(ids.option + i as u64);
            let mut option = Node::new(Role::ListBoxOption);
            option.set_label(match row {
                Row::Hit(h) => h.label(),
                Row::More(m) => more_text(*m).to_owned(),
            });
            option.set_selected(i == active);
            option.set_position_in_set(i + 1);
            option.set_size_of_set(rows.len());
            option.add_action(Action::Click);
            node.push_child(id);
            options.push((id, option));
        }
        list.push((ids.list, node));
        list.extend(options);
    }
    // A polite live region: the screen reader announces each state change.
    let mut line = Node::new(Role::Status);
    line.set_live(Live::Polite);
    if let Some(status) = status {
        line.set_value(status.text());
    } else if hits > 0 {
        line.set_value(format!("{hits} risultati"));
    }
    Projection {
        list,
        status: (ids.status, line),
        active: (active < rows.len()).then(|| NodeId(ids.option + active as u64)),
    }
}

/// The row a click on `node` means, if it is one of `len` options.
pub fn index_of(ids: Ids, node: NodeId, len: usize) -> Option<usize> {
    let i = node.0.checked_sub(ids.option)? as usize;
    (i < len).then_some(i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_engine::search::More;

    struct Room;
    impl Hit for Room {
        fn key(&self) -> &str {
            "!r"
        }
        fn title(&self) -> &str {
            "Cucina"
        }
        fn detail(&self) -> String {
            "#cucina:s · 12 membri".into()
        }
        fn avatar(&self) -> Option<&str> {
            None
        }
    }
    const IDS: Ids = Ids {
        list: NodeId(1),
        status: NodeId(2),
        option: 100,
    };

    #[test]
    fn options_read_name_alias_members_and_more_is_the_last_option() {
        let room = Room;
        let rows = [Row::Hit(&room), Row::More(More::Available)];
        let p = project(IDS, &rows, 1, None);
        let node = |id: u64| &p.list.iter().find(|(i, _)| *i == NodeId(id)).unwrap().1;
        assert_eq!(node(1).role(), Role::ListBox);
        assert_eq!(node(100).label(), Some("Cucina, #cucina:s · 12 membri"));
        assert_eq!(node(101).label(), Some("Altri risultati"));
        assert_eq!(node(101).is_selected(), Some(true));
        assert_eq!(node(101).size_of_set(), Some(2));
        assert_eq!(p.active, Some(NodeId(101)));
        assert_eq!(p.status.1.value(), Some("1 risultati"));
        assert_eq!(index_of(IDS, NodeId(101), 2), Some(1));
        assert_eq!(index_of(IDS, NodeId(102), 2), None);
        assert_eq!(index_of(IDS, NodeId(2), 2), None);
    }

    #[test]
    fn no_rows_no_listbox() {
        let p = project::<Room>(IDS, &[], 0, Some(Status::NoResults));
        assert!(p.list.is_empty() && p.active.is_none());
        assert_eq!(p.status.1.value(), Some("Nessun risultato"));
    }
}
