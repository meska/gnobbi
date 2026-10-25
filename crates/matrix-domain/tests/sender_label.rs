use matrix_domain::shows_sender_label;

const MIN: u64 = 60_000;

// (member_count, outgoing, prev sender, same sender id, gap ms) -> expected
#[test]
fn sender_label_table() {
    // (members, outgoing, previous: Option<(same_sender, gap_ms)>, expected)
    let cases: &[(u32, bool, Option<(bool, u64)>, bool, &str)] = &[
        (2, false, None, false, "dm first"),
        (2, false, Some((false, 10 * MIN)), false, "dm other sender"),
        (3, false, None, true, "group first"),
        (3, false, Some((false, MIN)), true, "group sender changes"),
        (3, false, Some((true, MIN)), false, "group consecutive"),
        (3, false, Some((true, 10 * MIN)), true, "group same sender after long gap"),
        (3, true, None, false, "own message"),
        (3, true, Some((false, MIN)), false, "own after other"),
        (1, false, None, false, "single member"),
    ];
    for (members, outgoing, prev, expected, name) in cases {
        assert_eq!(shows_sender_label(*members, *outgoing, *prev), *expected, "{name}");
    }
}
