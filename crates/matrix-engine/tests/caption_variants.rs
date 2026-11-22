//! Caption variants: what ruma's media caption accessors return for each
//! raw event shape the disposable Synapse/Tuwunel probe stored as fixtures
//! using the caption_variants script. Reception code today
//! (room_sync.rs message_label/message_rich) reads none of these, so this test
//! is the reusable contract for the caption feature: for every variant and
//! media msgtype, filename()/caption()/formatted_caption() must match the spec
//! table below. Offline: parses the committed fixture JSON, no server needed.
#![cfg(feature = "sdk")]
use matrix_engine::sdk::ruma::events::room::message::{MessageFormat, MessageType, RoomMessageEventContent};
use std::path::PathBuf;

/// (expected filename(), caption() is-some, formatted_caption() is-some)
fn expect(variant: &str, msgtype: &str) -> (String, bool, bool) {
    match variant {
        // MSC2530: filename != body => body is the caption, formatted too.
        "a" => (format!("sent-{msgtype}.bin"), true, true),
        // Legacy: body is the file name, no filename field => no caption.
        "b" => (format!("sent-{msgtype}.bin"), false, false),
        // body = "$event-id-like", caption only in formatted_body, no filename
        // => ruma exposes NOTHING: both accessors are None.
        "c" => ("$0FGh_dsG1uK8M9sqaEGIMiTdbN4OsP0j5mSZvwR8wcU".into(), false, false),
        // filename == body => not a caption per spec.
        "d" => (format!("sent-{msgtype}.bin"), false, false),
        other => panic!("unknown variant {other}"),
    }
}

#[test]
fn caption_variants_match_spec_table() {
    let dir = std::env::var("FIXTURE_CAPTIONS_DIR").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/captions")
    });
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("fixtures dir {:?}: {e}", dir))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    assert_eq!(files.len(), 32, "expected 16 fixtures x 2 servers in {dir:?}");
    println!("server -- variant -- msgtype | filename() | caption() | formatted_caption()");
    for path in files {
        let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let (server, variant, msgtype) = match name.split("--").collect::<Vec<_>>()[..] {
            [s, v, m] => (s.to_owned(), v.to_owned(), m.to_owned()),
            _ => panic!("bad fixture name {name}"),
        };
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        // The event must have survived the server round-trip with our fields intact.
        for key in match variant.as_str() {
            "a" => vec!["body", "filename", "formatted_body"],
            "b" => vec!["body"],
            "c" => vec!["body", "formatted_body"],
            _ => vec!["body", "filename"],
        } {
            assert!(raw["content"].get(key).is_some(), "{name}: {key} lost by {server}");
        }
        if variant == "b" || variant == "c" {
            assert!(raw["content"].get("filename").is_none(), "{name}: filename added by {server}");
        }
        let content: RoomMessageEventContent =
            serde_json::from_value(raw["content"].clone()).unwrap();
        let (filename, caption, formatted) = match &content.msgtype {
            MessageType::Image(m) => (m.filename().to_owned(), m.caption().map(str::to_owned), m.formatted_caption().map(|f| (f.format.clone(), f.body.clone()))),
            MessageType::File(m) => (m.filename().to_owned(), m.caption().map(str::to_owned), m.formatted_caption().map(|f| (f.format.clone(), f.body.clone()))),
            MessageType::Video(m) => (m.filename().to_owned(), m.caption().map(str::to_owned), m.formatted_caption().map(|f| (f.format.clone(), f.body.clone()))),
            MessageType::Audio(m) => (m.filename().to_owned(), m.caption().map(str::to_owned), m.formatted_caption().map(|f| (f.format.clone(), f.body.clone()))),
            other => panic!("{name}: unexpected msgtype {}", other.msgtype()),
        };
        let (want_name, want_caption, want_formatted) = expect(&variant, &msgtype);
        assert_eq!(filename, want_name, "{server} {variant} {msgtype}: filename()");
        assert_eq!(caption.is_some(), want_caption, "{server} {variant} {msgtype}: caption() = {caption:?}");
        assert_eq!(formatted.is_some(), want_formatted, "{server} {variant} {msgtype}: formatted_caption()");
        if let Some((format, _)) = &formatted {
            assert_eq!(*format, MessageFormat::Html);
        }
        println!("{server} -- {variant} -- {msgtype} | {filename} | {caption:?} | {}", formatted.is_some());
    }
}
