//! `order` keys of `m.space.child`: strings of 0x20..=0x7E, at most 50 characters, compared lexicographically.
//! Children without a key sort after the keyed ones.

const MAX: usize = 50;

/// A key strictly between `a` (`""` = before everything) and `b` (`None` = after everything).
/// `None` when there is no room: `a >= b`, nothing fits (`"x"` and `"x "`), or it would pass 50 characters.
pub fn between(a: &str, b: Option<&str>) -> Option<String> {
    if b.is_some_and(|b| a >= b) {
        return None;
    }
    let (a, b) = (a.as_bytes(), b.map(str::as_bytes));
    // `capped`: the key so far equals the start of `b`, so the next character is bounded by `b` too.
    let (mut out, mut capped) = (Vec::new(), b.is_some());
    for i in 0..MAX {
        // 0x1F / 0x7F: one outside the alphabet, for an `a` that ended and for no upper bound.
        let lo = a.get(i).copied().unwrap_or(0x1F);
        let hi = if capped { *b?.get(i)? } else { 0x7F };
        if hi - lo >= 2 {
            out.push(lo + (hi - lo) / 2);
            return String::from_utf8(out).ok();
        }
        let c = lo.max(0x20);
        capped &= c == hi;
        out.push(c);
    }
    None
}

/// `n` distinct ascending keys, evenly spread.
// shortcut: two characters, so distinct only up to ~9000 children; widen the key when a space gets there.
fn spread(n: usize) -> impl Iterator<Item = String> {
    (1..=n).map(move |i| {
        let v = i * 95 * 95 / (n + 1);
        String::from_utf8(vec![0x20 + (v / 95) as u8, 0x20 + (v % 95) as u8]).unwrap_or_default()
    })
}

/// Keys to write so that `moved` sits right before `before` (`None` = last) among `siblings`, the children in
/// their current order with their key. One write when a key fits between the neighbours; otherwise the
/// children up to the moved one (and every keyed one) get fresh keys. `moved` is always the first write.
pub fn plan(siblings: &[(String, Option<String>)], moved: &str, before: Option<&str>) -> Vec<(String, String)> {
    let rest: Vec<&(String, Option<String>)> = siblings.iter().filter(|(id, _)| id != moved).collect();
    let at = before.and_then(|b| rest.iter().position(|(id, _)| id == b)).unwrap_or(rest.len());
    // A neighbour above without a key sorts by age, not by key: no single key puts `moved` after it.
    let above = if at == 0 { Some("") } else { rest[at - 1].1.as_deref() };
    if let Some(key) = above.and_then(|a| between(a, rest.get(at).and_then(|(_, key)| key.as_deref()))) {
        return vec![(moved.to_owned(), key)];
    }
    let mut all: Vec<(&str, Option<&str>)> = rest.iter().map(|(id, key)| (id.as_str(), key.as_deref())).collect();
    all.insert(at, (moved, None));
    let n = all.iter().rposition(|(_, key)| key.is_some()).map_or(at, |last| last.max(at)) + 1;
    let mut writes: Vec<(String, String)> =
        all.iter().zip(spread(n)).filter(|((_, old), new)| *old != Some(new.as_str())).map(|((id, _), new)| ((*id).to_owned(), new)).collect();
    writes.sort_by_key(|(id, _)| id != moved); // stable
    writes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(key: &str) -> bool {
        !key.is_empty() && key.len() <= MAX && key.bytes().all(|b| (0x20..=0x7E).contains(&b))
    }

    #[test]
    fn a_key_between_two_others_sorts_between_them_and_stays_valid() {
        for (a, b) in [("", None), ("", Some("!")), ("a", Some("b")), ("a", Some("a!")), ("a~", Some("b")), ("~~", None), ("az", Some("b")), ("", Some("  !"))] {
            let key = between(a, b).unwrap_or_else(|| panic!("no key between {a:?} and {b:?}"));
            assert!(valid(&key) && key.as_str() > a && b.is_none_or(|b| key.as_str() < b), "{a:?} < {key:?} < {b:?}");
        }
    }

    #[test]
    fn no_key_where_none_fits() {
        assert_eq!(between("b", Some("a")), None);
        assert_eq!(between("a", Some("a")), None);
        assert_eq!(between("a", Some("a ")), None, "nothing sorts between a string and itself plus a space");
        assert_eq!(between("", Some(" ")), None);
        assert_eq!(between(&"~".repeat(50), None), None, "50 characters at most");
    }

    #[test]
    fn appending_many_times_keeps_keys_ascending_and_short() {
        let mut last = String::new();
        for _ in 0..200 {
            let next = between(&last, None).unwrap();
            assert!(valid(&next) && next > last);
            last = next;
        }
        assert!(last.len() <= 40, "{}", last.len());
    }

    fn kids(list: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
        list.iter().map(|(id, key)| ((*id).to_owned(), key.map(str::to_owned))).collect()
    }

    /// Ids in the order the spec gives after `writes`: keyed first by key, the others in their old sequence (age).
    fn applied(siblings: &[(String, Option<String>)], moved: &str, writes: &[(String, String)]) -> Vec<String> {
        let mut all = siblings.to_vec();
        if !all.iter().any(|(id, _)| id == moved) {
            all.push((moved.to_owned(), None));
        }
        for (id, key) in writes {
            all.iter_mut().find(|(i, _)| i == id).unwrap().1 = Some(key.clone());
        }
        let mut order: Vec<usize> = (0..all.len()).collect();
        order.sort_by(|&x, &y| match (&all[x].1, &all[y].1) {
            (Some(a), Some(b)) => a.cmp(b).then(x.cmp(&y)),
            (a, b) => b.is_some().cmp(&a.is_some()).then(x.cmp(&y)),
        });
        order.into_iter().map(|i| all[i].0.clone()).collect()
    }

    #[test]
    fn one_write_when_a_key_fits_between_the_neighbours() {
        let s = kids(&[("a", Some("A")), ("b", Some("M")), ("c", None)]);
        for (moved, before, want) in [("x", Some("b"), ["a", "x", "b", "c"]), ("x", Some("a"), ["x", "a", "b", "c"]), ("x", Some("c"), ["a", "b", "x", "c"])] {
            let writes = plan(&s, moved, before);
            assert_eq!(writes.len(), 1, "{writes:?}");
            assert_eq!(applied(&s, moved, &writes), want);
        }
        // Reorder inside the same parent: the moved child is taken out first.
        let writes = plan(&s, "b", Some("a"));
        assert_eq!((writes.len(), applied(&s, "b", &writes)), (1, vec!["b".to_owned(), "a".into(), "c".into()]));
        // An empty space: the first child still gets a key.
        assert_eq!(plan(&[], "x", None).len(), 1);
    }

    #[test]
    fn children_without_key_above_the_spot_get_keys_in_block() {
        let s = kids(&[("a", Some("A")), ("b", None), ("c", None), ("d", None)]);
        let writes = plan(&s, "x", Some("d"));
        assert_eq!(writes[0].0, "x", "the moved child is written first");
        assert_eq!(applied(&s, "x", &writes), ["a", "b", "c", "x", "d"]);
        assert!(!writes.iter().any(|(id, _)| id == "d"), "children below the spot keep no key");
        // Last of all, nothing keyed.
        let s = kids(&[("a", None), ("b", None)]);
        assert_eq!(applied(&s, "x", &plan(&s, "x", None)), ["a", "b", "x"]);
        assert_eq!(applied(&s, "a", &plan(&s, "a", None)), ["b", "a"]);
    }

    #[test]
    fn equal_or_adjacent_keys_are_reassigned_in_block() {
        let s = kids(&[("a", Some("k")), ("b", Some("k")), ("c", Some("k ")), ("d", None)]);
        for before in ["b", "c"] {
            let writes = plan(&s, "x", Some(before));
            assert!(writes.iter().all(|(_, key)| valid(key)), "{writes:?}");
            let want: Vec<&str> = if before == "b" { vec!["a", "x", "b", "c", "d"] } else { vec!["a", "b", "x", "c", "d"] };
            assert_eq!(applied(&s, "x", &writes), want);
        }
    }
}
