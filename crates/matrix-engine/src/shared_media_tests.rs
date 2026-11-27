use super::*;
use serde_json::json;
use std::cell::Cell;

fn msg(id: &str, content: Value) -> Value {
    json!({"type":"m.room.message","event_id":id,"sender":"@al:s","origin_server_ts":10,"content":content})
}

fn names(items: &[SharedItem]) -> Vec<&str> {
    items.iter().map(|i| i.event_id.as_str()).collect()
}

#[test]
fn each_tab_takes_only_its_own_messages() {
    let events = [
        msg("$img", json!({"msgtype":"m.image","body":"a.png","url":"mxc://s/a","info":{"w":4,"h":3}})),
        msg("$vid", json!({"msgtype":"m.video","body":"v.mp4","url":"mxc://s/v","info":{"duration":65000}})),
        msg("$pdf", json!({"msgtype":"m.file","body":"caption","filename":"doc.pdf","url":"mxc://s/d","info":{"size":2048}})),
        msg("$voice", json!({"msgtype":"m.audio","body":"voice","url":"mxc://s/o","org.matrix.msc3245.voice":{}})),
        msg("$song", json!({"msgtype":"m.audio","body":"song.mp3","url":"mxc://s/s"})),
        msg("$link", json!({"msgtype":"m.text","body":"guarda (https://example.org/x?a=1)."})),
        msg("$text", json!({"msgtype":"m.text","body":"niente da vedere"})),
        json!({"type":"m.room.member","event_id":"$m","sender":"@al:s","origin_server_ts":1,"content":{}}),
    ];
    let of = |tab| events.iter().filter_map(|e| classify(tab, e)).collect::<Vec<_>>();
    let media = of(SharedTab::Media);
    assert_eq!(names(&media), ["$img", "$vid"]);
    assert_eq!((media[0].kind, media[1].kind, media[1].duration_ms), (SharedKind::Image, SharedKind::Video, Some(65000)));
    let files = of(SharedTab::Files);
    assert_eq!(names(&files), ["$pdf", "$song"]);
    assert_eq!((files[0].name.as_str(), files[0].size), ("doc.pdf", Some(2048)));
    let links = of(SharedTab::Links);
    assert_eq!(names(&links), ["$link"]);
    assert_eq!(links[0].url.as_deref(), Some("https://example.org/x?a=1"));
}

#[test]
fn redacted_messages_and_edits_are_never_listed() {
    let mut gone = msg("$gone", json!({"msgtype":"m.image","body":"a.png"}));
    gone["unsigned"] = json!({"redacted_because": {"type": "m.room.redaction"}});
    let edit = msg("$edit", json!({"msgtype":"m.text","body":"* https://a.example","m.relates_to":{"rel_type":"m.replace","event_id":"$x"}}));
    assert!(classify(SharedTab::Media, &gone).is_none());
    assert!(classify(SharedTab::Links, &edit).is_none());
}

#[test]
fn urls_need_a_scheme_and_a_host() {
    assert_eq!(first_url("vedi HTTPS://Example.org/p, ok"), Some("HTTPS://Example.org/p"));
    assert_eq!(first_url("<http://a.example>"), Some("http://a.example"));
    assert_eq!(first_url("http:// vuoto e www.example.org e ftp://x.example"), None);
    assert_eq!(first_url("è così: àèìòù"), None);
}

#[test]
fn only_a_plain_room_lets_the_server_filter() {
    let plain = filter(SharedTab::Media, false);
    assert_eq!(plain.types, Some(vec!["m.room.message".to_owned()]));
    assert_eq!(plain.url_filter, Some(UrlFilter::EventsWithUrl));
    assert_eq!(filter(SharedTab::Files, false).url_filter, Some(UrlFilter::EventsWithUrl));
    // A link has no `url` key: the body is read here.
    let links = filter(SharedTab::Links, false);
    assert_eq!((links.types.is_some(), links.url_filter), (true, None));
    let sealed = filter(SharedTab::Media, true);
    assert_eq!((sealed.types, sealed.url_filter), (None, None));
}

fn text_page(i: usize) -> Vec<Value> {
    vec![msg(&format!("$t{i}"), json!({"msgtype":"m.text","body":"solo testo"}))]
}

#[tokio::test]
async fn a_pass_reads_at_most_ten_pages_when_nothing_matches() {
    let calls = Cell::new(0);
    let (items, next, done) = pass(SharedTab::Media, None, |from| {
        calls.set(calls.get() + 1);
        let n = calls.get();
        async move {
            assert_eq!(from, (n > 1).then(|| format!("t{}", n - 1)));
            Ok((text_page(n), Some(format!("t{n}"))))
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.get(), PASS_PAGES);
    // The next pass starts where this one stopped.
    assert_eq!((items.len(), next.as_deref(), done), (0, Some("t10"), false));
}

#[tokio::test]
async fn a_pass_ends_with_the_room_and_stops_early_when_it_has_enough() {
    let calls = Cell::new(0);
    let (items, next, done) = pass(SharedTab::Media, None, |_| {
        calls.set(calls.get() + 1);
        let last = calls.get() == 3;
        async move { Ok((vec![msg("$i", json!({"msgtype":"m.image","body":"a.png"}))], (!last).then(|| "n".to_owned()))) }
    })
    .await
    .unwrap();
    assert_eq!((calls.get(), items.len(), next, done), (3, 3, None, true));

    let calls = Cell::new(0);
    let (items, _, done) = pass(SharedTab::Media, None, |_| {
        calls.set(calls.get() + 1);
        async move { Ok(((0..20).map(|i| msg(&format!("$i{i}"), json!({"msgtype":"m.image","body":"a.png"}))).collect(), Some("n".to_owned()))) }
    })
    .await
    .unwrap();
    assert_eq!((calls.get(), items.len(), done), (2, 40, false));
}

#[tokio::test]
async fn a_failed_page_fails_the_pass() {
    assert!(pass(SharedTab::Media, None, |_| async { Err(()) }).await.is_err());
}
