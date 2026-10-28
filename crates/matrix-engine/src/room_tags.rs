//! Room groups from the standard Matrix room tags (`m.favourite`, `m.lowpriority`, `u.*` with
//! `order`), as the SDK keeps them from /sync. Read-only: nothing here writes to the server, so
//! another device changing a tag shows up here on the next sync and nowhere else is stored.
use matrix_sdk::{Client, ruma::events::tag::{TagName, Tags}};
use std::collections::BTreeMap;

/// Rooms of one tag, in the order the tags ask for.
pub type TagRooms = Vec<String>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomGroups {
    pub favourites: TagRooms,
    pub low_priority: TagRooms,
    /// Keyed by the full tag name, `u.` prefix included.
    pub user: BTreeMap<String, TagRooms>,
}

/// Names of the tags that are groups (`m.favourite`, `m.lowpriority`, `u.*`), sorted.
pub fn group_names(tags: &Tags) -> Vec<String> {
    let mut names: Vec<String> = tags.keys().filter(|n| is_group(n)).map(|n| n.to_string()).collect();
    names.sort();
    names
}

// Other tags (m.server_notice, unknown) are not groups.
fn is_group(name: &TagName) -> bool {
    matches!(name, TagName::Favorite | TagName::LowPriority | TagName::User(_))
}

impl RoomGroups {
    /// Lowest `order` first; rooms without one follow, then ties break by room id (stable per spec).
    pub fn from_tags<'a>(rooms: impl IntoIterator<Item = (&'a str, &'a Tags)>) -> Self {
        let mut buckets: BTreeMap<String, Vec<(Option<f64>, &str)>> = BTreeMap::new();
        for (room, tags) in rooms {
            for (name, info) in tags {
                if is_group(name) {
                    buckets.entry(name.to_string()).or_default().push((info.order.filter(|o| o.is_finite()), room));
                }
            }
        }
        let sorted = |mut v: Vec<(Option<f64>, &str)>| -> TagRooms {
            v.sort_by(|a, b| match (a.0, b.0) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }.then_with(|| a.1.cmp(b.1)));
            v.into_iter().map(|(_, r)| r.to_owned()).collect()
        };
        let mut groups = Self::default();
        for (name, rooms) in buckets {
            match name.as_str() {
                "m.favourite" => groups.favourites = sorted(rooms),
                "m.lowpriority" => groups.low_priority = sorted(rooms),
                _ => { groups.user.insert(name, sorted(rooms)); }
            }
        }
        groups
    }

    /// Reads the tags of every joined room from the SDK store (filled by /sync); no request is sent.
    pub async fn read(client: &Client) -> Result<Self, matrix_sdk::Error> {
        let mut all = Vec::new();
        for room in client.joined_rooms() {
            if let Some(tags) = room.tags().await? { all.push((room.room_id().to_string(), tags)); }
        }
        Ok(Self::from_tags(all.iter().map(|(r, t)| (r.as_str(), t))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::events::tag::TagInfo;

    fn tags(list: &[(&str, Option<f64>)]) -> Tags {
        list.iter().map(|(n, o)| { let mut i = TagInfo::new(); i.order = *o; (TagName::from(*n), i) }).collect()
    }

    #[test]
    fn group_names_keep_only_groups_sorted() {
        let t = tags(&[("u.work", None), ("m.server_notice", None), ("m.favourite", None), ("m.weird", None)]);
        assert_eq!(group_names(&t), ["m.favourite", "u.work"]);
    }

    #[test]
    fn groups_are_ordered_by_order_then_missing_then_id() {
        let a = tags(&[("m.favourite", Some(0.9)), ("u.work", None)]);
        let b = tags(&[("m.favourite", Some(0.1)), ("m.lowpriority", None), ("m.server_notice", None)]);
        let c = tags(&[("m.favourite", None), ("u.work", Some(0.5))]);
        let d = tags(&[("m.favourite", None), ("m.weird", Some(1.0))]);
        let g = RoomGroups::from_tags([("!a", &a), ("!b", &b), ("!c", &c), ("!d", &d)]);
        assert_eq!(g.favourites, ["!b", "!a", "!c", "!d"]);
        assert_eq!(g.low_priority, ["!b"]);
        assert_eq!(g.user["u.work"], ["!c", "!a"]);
        assert_eq!(g.user.len(), 1);
    }
}
