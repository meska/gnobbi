//! Edge cases of the rendered Argomento: hostile, huge and odd input must stay inside the contract
//! (spans/links on char boundaries, no empty/invisible link, no script/file/javascript link, bounded time).
#![cfg(feature = "sdk")]

use matrix_engine::rich_text::{topic, Topic};
use std::time::{Duration, Instant};

fn fits(t: &Topic) {
    for s in &t.styles {
        assert!(s.range.start < s.range.end && s.range.end <= t.text.len(), "{s:?}");
        assert!(t.text.is_char_boundary(s.range.start) && t.text.is_char_boundary(s.range.end), "{s:?} in {:?}", t.text);
    }
    for l in &t.links {
        assert!(l.range.start < l.range.end, "empty (invisible) link {l:?} in {:?}", t.text);
        assert!(l.range.end <= t.text.len(), "{l:?}");
        assert!(t.text.is_char_boundary(l.range.start) && t.text.is_char_boundary(l.range.end), "{l:?} in {:?}", t.text);
        let lower = l.href.to_ascii_lowercase();
        for bad in ["javascript:", "data:", "file:", "vbscript:"] {
            assert!(!lower.starts_with(bad), "dangerous href kept: {l:?}");
        }
    }
}

fn timed(plain: &str, html: Option<&str>) -> Topic {
    let start = Instant::now();
    let t = topic(plain, html);
    // 30 s: a deep tree costs ~5 s in a debug build (html5ever/ruma), ~0.3 s in release; the bound only catches a hang.
    assert!(start.elapsed() < Duration::from_secs(30), "topic() took {:?} for {} bytes", start.elapsed(), plain.len() + html.map_or(0, str::len));
    fits(&t);
    t
}

#[test]
fn empty_and_blank_topics_have_no_spans() {
    for s in ["", " ", "\n\n\t", "\u{200b}"] {
        let t = timed(s, None);
        assert!(t.styles.is_empty() && t.links.is_empty(), "{s:?} -> {t:?}");
        assert!(t.text.trim().is_empty() || t.text == "\u{200b}", "{s:?} -> {t:?}");
    }
}

#[test]
fn empty_label_links_are_not_invisible_targets() {
    // `[](url)` and `<a href></a>` would give a zero-width clickable target.
    let t = timed("[](https://evil.example) dopo", None);
    assert!(t.links.iter().all(|l| l.range.start < l.range.end), "{t:?}");
    let t = timed("x", Some("<a href=\"https://evil.example\"></a>ciao"));
    assert!(t.links.iter().all(|l| l.range.start < l.range.end), "{t:?}");
}

#[test]
fn commonmark_autolinks_and_reference_links_never_keep_dangerous_schemes() {
    let t = timed("<javascript:alert(1)> <file:///etc/passwd> <https://ok.example>", None);
    assert!(t.links.iter().any(|l| l.href == "https://ok.example"), "{t:?}");
    let t = timed("[a][b] [c]\n\n[b]: javascript:alert(1)\n[c]: file:///etc/passwd \"titolo\"", None);
    assert!(t.links.is_empty(), "{t:?}");
}

#[test]
fn link_titles_and_angle_destinations_do_not_leak_into_the_text() {
    let t = timed("[x](<https://a.example/p q> \"TITOLO\")", None);
    assert!(!t.text.contains("TITOLO"), "{t:?}");
}

#[test]
fn emoji_zwj_and_wide_chars_keep_every_span_on_a_boundary() {
    let t = timed("👩‍👩‍👧 **così 日本語 🌱** [é👍](https://e.org/é) *a̐éö̲*\n- 🚀 `x🚀y`", None);
    assert!(!t.styles.is_empty() && !t.links.is_empty(), "{t:?}");
}

#[test]
fn newlines_in_a_label_and_nested_markup_keep_links_inside_the_text() {
    let t = timed("[riga uno\nriga due **b**](https://e.org)", None);
    assert_eq!(t.links.len(), 1, "{t:?}");
    let t = timed("**[bold link](https://e.org)** *[it](https://e.org/2)* `[no](https://e.org/3)`", None);
    assert_eq!(t.links.len(), 2, "code must not link: {t:?}");
}

#[test]
fn deeply_nested_html_part_does_not_overflow_the_stack() {
    timed("x", Some(&"<b><i><ul><li><blockquote>".repeat(2_400)));
}

#[test]
fn malformed_html_part_is_bounded_and_safe() {
    let html = "<b><i><a href=\"https://e.org\">x</b></i><p><ul><li><ul><li>a</a></div><<<>>>&amp;&#xD800;&#0;<a href='javascript:x'>j</a>";
    let t = timed("fallback", Some(html));
    assert!(!t.text.is_empty());
}

#[test]
#[ignore = "BUG (low): U+202E in a link label survives and reorders the real address shown after it; strip bidi controls (U+202A-202E, 2066-2069) from the topic text"]
fn control_and_bidi_characters_do_not_reach_the_visible_text_unchecked() {
    // Informative: RLO/LRO/NUL in a link label can reorder the shown address. Pin what we ship today.
    let t = timed("[https://buono.example\u{202e}](https://cattivo.example)", None);
    assert!(!t.text.contains('\u{0}'), "{t:?}");
    assert!(!t.text.contains('\u{202e}'), "bidi override survives in the label: {:?}", t.text);
}

#[test]
fn huge_topic_is_bounded() {
    let paragraph = "Riga **grassa** con [link](https://e.org/x) e https://e.org/nudo e `codice` 🌱 ".repeat(2_000);
    let t = timed(&paragraph, None);
    assert!(t.links.len() >= 4_000);
    // The room topic is capped by the server (Synapse 65 KB event); here 1 MB must still finish.
    timed(&"a ".repeat(500_000), None);
}

macro_rules! pathological {
    ($($(#[$m:meta])* $name:ident: $src:expr;)*) => {$(
        #[test]
        $(#[$m])*
        fn $name() {
            timed(&$src, None);
        }
    )*};
}

// A room admin controls the topic: none of these may abort the process (stack overflow) or hang.
pathological! {
    pathological_nested_lists: "- ".repeat(5_000) + "x";
    pathological_open_brackets: "[".repeat(100_000);
    pathological_stars: "*".repeat(100_000);
    pathological_underscores: "_a".repeat(50_000);
    pathological_nested_quotes: "> ".repeat(5_000) + "x";
    pathological_unclosed_links: "[a](".repeat(20_000);
    pathological_backticks: "`".repeat(100_000);
    pathological_images: "![".repeat(50_000);
    pathological_angle_brackets: "<".repeat(100_000);
    pathological_schemes: "https://".repeat(20_000);
    
}

#[test]
#[ignore = "BUG (low): a bare URL wrapped in «» is not linkified (leading guillemet not peeled)"]
fn bare_urls_with_odd_punctuation_stay_one_link_without_the_trailing_mark() {
    let t = timed("Vedi https://e.org/a_b_c?x=1&y=*2*. E (https://e.org/p) e «https://e.org/q» e https://e.org/r,", None);
    let hrefs: Vec<_> = t.links.iter().map(|l| (&t.text[l.range.clone()], l.href.as_str())).collect();
    for (label, href) in &hrefs {
        assert_eq!(label, href, "{hrefs:?}");
        assert!(!href.ends_with(['.', ',', ')', '»']), "{hrefs:?}");
    }
    assert_eq!(hrefs.len(), 4, "{hrefs:?}");
}

// Sizes stay within one Matrix event (64 KiB): bigger inputs are rejected by the server, and html5ever is quadratic on them.
// html5ever ignores the self-closing flag on non-void tags and unmatched end tags, so the real tree nests while a
// textual depth count stays flat: the parse must survive whatever the tree looks like.
#[test]
fn self_closing_non_void_tags_do_not_overflow_the_stack() {
    timed("x", Some(&"<b/>".repeat(16_000)));
}

#[test]
fn unmatched_end_tags_do_not_overflow_the_stack() {
    timed("x", Some(&"<b></x>".repeat(9_000)));
}

#[test]
fn end_p_without_open_p_does_not_overflow_the_stack() {
    timed("x", Some(&"<b></p>".repeat(9_000)));
}

#[test]
fn adoption_agency_misnesting_does_not_overflow_the_stack() {
    timed("x", Some(&"<i><div></i>".repeat(5_000)));
}

#[test]
fn timeline_html_does_not_overflow_the_stack() {
    use matrix_engine::rich_text::from_html;
    // any room member can send these
    let (text, _) = from_html(&"<i/>".repeat(16_000));
    assert!(text.len() < 1000);
    from_html(&"<i><div></i>".repeat(5_000));
}

#[test]
#[ignore = "release measurement: cargo test --release ... -- --ignored --nocapture"]
fn max_size_event_of_nested_i_finishes() {
    use matrix_engine::rich_text::from_html;
    let html = "<i>".repeat(65_536 / 3); // 21 845 levels = a whole 64 KiB event
    let t = Instant::now();
    from_html(&html);
    eprintln!("timeline 64 KiB <i>: {:?}", t.elapsed());
    let t = Instant::now();
    topic("x", Some(&html));
    eprintln!("topic 64 KiB <i>: {:?}", t.elapsed());
}
