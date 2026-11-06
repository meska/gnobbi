use super::*;
use serde_json::json;

fn msg(id: &str, sender: &str, ts: u64, body: &str) -> Value {
    json!({"type":"m.room.message","event_id":id,"sender":sender,"origin_server_ts":ts,"content":{"msgtype":"m.text","body":body}})
}

fn edit(id: &str, of: &str, sender: &str, ts: u64, body: &str) -> Value {
    json!({"type":"m.room.message","event_id":id,"sender":sender,"origin_server_ts":ts,"content":{
        "msgtype":"m.text","body":format!("* {body}"),"m.new_content":{"msgtype":"m.text","body":body},
        "m.relates_to":{"rel_type":"m.replace","event_id":of}}})
}

fn ids(hits: &[MessageHit]) -> Vec<&str> {
    hits.iter().map(|h| h.event_id.as_str()).collect()
}

#[test]
fn ranges_ignore_case_and_accents_and_are_byte_ranges_of_the_text() {
    let text = "Perché è già CAFÉ, cafe";
    let found = find_ranges(text, "cafe");
    let words: Vec<&str> = found.iter().map(|r| &text[r.clone()]).collect();
    assert_eq!(words, ["CAFÉ", "cafe"]);
    assert_eq!(find_ranges(text, "PERCHE").len(), 1);
    assert!(find_ranges(text, "").is_empty());
    assert!(find_ranges(text, "zebra").is_empty());
}

#[test]
fn local_hits_use_the_latest_edit_drop_redacted_and_sort_newest_first() {
    let events = vec![
        msg("$a", "@al:s", 10, "ciao mondo"),
        msg("$b", "@al:s", 20, "altro testo"),
        edit("$e1", "$b", "@al:s", 25, "ciao dopo la modifica"),
        msg("$c", "@bo:s", 30, "ciao da bob"),
        json!({"type":"m.room.redaction","event_id":"$r","sender":"@bo:s","redacts":"$c","origin_server_ts":31,"content":{}}),
        msg("$d", "@bo:s", 40, "CIAO"),
        // An edit by someone else is not an edit: the original text stays.
        edit("$e2", "$d", "@al:s", 41, "niente"),
        json!({"type":"m.room.message","event_id":"$x","sender":"@al:s","origin_server_ts":50,"content":{},
               "unsigned":{"redacted_because":{"type":"m.room.redaction"}}}),
    ];
    let hits = local_hits(&events, "ciao");
    assert_eq!(ids(&hits), ["$d", "$b", "$a"]);
    assert_eq!(hits[1].excerpt, "ciao dopo la modifica");
    assert_eq!(hits[1].matches.len(), 1);
    assert_eq!(hits[1].matches[0], 0..4);
    // The old text of an edited message no longer matches.
    assert!(local_hits(&events, "altro").is_empty());
}

#[test]
fn local_hits_mark_thread_replies_and_flatten_lines() {
    let mut reply = msg("$t", "@al:s", 5, "riga uno\nriga due");
    reply["content"]["m.relates_to"] = json!({"rel_type":"m.thread","event_id":"$root"});
    let hits = local_hits(&[reply], "due");
    assert_eq!(hits[0].thread_root.as_deref(), Some("$root"));
    assert_eq!(hits[0].excerpt, "riga uno riga due");
    assert_eq!(hits[0].matches.len(), 1);
    assert_eq!(hits[0].matches[0], 14..17);
}

#[test]
fn long_bodies_are_cut_around_the_first_match() {
    let body = format!("{} ago {}", "x ".repeat(300), "y ".repeat(300));
    let hit = &local_hits(&[msg("$a", "@al:s", 1, &body)], "ago")[0];
    assert!(hit.excerpt.starts_with('…'));
    assert!(hit.excerpt.chars().count() <= EXCERPT_CHARS + 1);
    assert_eq!(&hit.excerpt[hit.matches[0].clone()], "ago");
}

#[test]
fn server_hits_keep_server_order_skip_edits_and_dedupe() {
    // The server stems («corsa» for «correre»): no local «contains» check on its results.
    let results = vec![
        msg("$2", "@al:s", 20, "ho fatto una corsa"),
        edit("$e", "$2", "@al:s", 21, "ho fatto una corsa lunga"),
        msg("$1", "@al:s", 10, "ho fatto una corsa"),
        msg("$2", "@al:s", 20, "ho fatto una corsa"),
    ];
    let hits = server_hits(&results, "correre");
    assert_eq!(ids(&hits), ["$2", "$1"]);
    assert!(hits[0].matches.is_empty());
}
