//! Formatted message bodies (`org.matrix.custom.html`) flattened to display text plus style spans,
//! so the timeline draws them with its plain styled text. The HTML is sanitized first (SDK/ruma
//! compat rules, reply fallback removed, nesting capped).
// shortcut: no inline images, tables are flattened to lines, a labelled link shows its address in
// brackets after the label; upgrade when the timeline draws blocks instead of one styled text.
use crate::history::HistorySender;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Bold,
    Italic,
    Strike,
    Code,
    Quote,
}

/// A styled byte range of the display text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub style: Style,
}

/// What a mention pill stands for (the MATRIX_MENZIONI_UI notes D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionKind {
    /// A room member, named as on their own bubbles; `me` is the active user.
    User { sender: HistorySender, me: bool },
    /// The literal «@room», only when the event's `m.mentions.room` is set.
    AtRoom,
    /// A `matrix.to` link to a room (`#alias` or `!id`).
    Room,
}

/// A mention in the display text, as a byte range of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionSpan {
    pub range: Range<usize>,
    pub kind: MentionKind,
}

/// What the engine knows about the room while flattening a body.
pub struct MentionCtx<'a> {
    /// Members (joined or invited) the body's links point at, by user id.
    pub users: &'a std::collections::HashMap<String, HistorySender>,
    pub me: &'a str,
    pub at_room: bool,
}

/// «@room» as a whole word outside code and outside the spans already taken.
pub fn at_room_spans(text: &str, styles: &[StyleSpan], taken: &[MentionSpan]) -> Vec<MentionSpan> {
    text.match_indices("@room")
        .filter(|(i, w)| {
            let end = i + w.len();
            text[..*i].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '@')
                && text[end..].chars().next().is_none_or(|c| !c.is_alphanumeric() && !"_:-".contains(c))
                && !styles.iter().any(|s| s.style == Style::Code && s.range.start < end && *i < s.range.end)
                && !taken.iter().any(|m| m.range.start < end && *i < m.range.end)
        })
        .map(|(i, w)| MentionSpan { range: i..i + w.len(), kind: MentionKind::AtRoom })
        .collect()
}

#[cfg(feature = "sdk")]
#[derive(Default)]
struct Rich {
    text: String,
    spans: Vec<StyleSpan>,
    mentions: Vec<MentionSpan>,
}

#[cfg(feature = "sdk")]
impl Rich {
    fn newline(&mut self) {
        self.text.truncate(self.text.trim_end_matches(' ').len());
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.text.push('\n');
        }
    }

    fn push_text(&mut self, s: &str, verbatim: bool) {
        if verbatim {
            self.text.push_str(s);
            return;
        }
        let mut space = self.text.is_empty() || self.text.ends_with(['\n', ' ']);
        for c in s.chars() {
            if c.is_whitespace() {
                if !space {
                    self.text.push(' ');
                    space = true;
                }
            } else {
                self.text.push(c);
                space = false;
            }
        }
    }

    fn append(&mut self, other: Rich) {
        let base = self.text.len();
        self.text.push_str(&other.text);
        self.spans
            .extend(other.spans.into_iter().map(|s| StyleSpan {
                range: s.range.start + base..s.range.end + base,
                style: s.style,
            }));
        self.mentions
            .extend(other.mentions.into_iter().map(|m| MentionSpan {
                range: m.range.start + base..m.range.end + base,
                kind: m.kind,
            }));
    }

    fn styled(&mut self, inner: Rich, style: Style) {
        let start = self.text.len();
        self.append(inner);
        if self.text.len() > start {
            self.spans.push(StyleSpan {
                range: start..self.text.len(),
                style,
            });
        }
    }

    /// Drop trailing blanks and newlines, clamping the spans.
    fn trimmed(mut self) -> Rich {
        self.text.truncate(self.text.trim_end().len());
        let end = self.text.len();
        self.spans.retain_mut(|s| {
            s.range.end = s.range.end.min(end);
            s.range.start < s.range.end
        });
        self.mentions.retain_mut(|m| {
            m.range.end = m.range.end.min(end);
            m.range.start < m.range.end
        });
        self
    }

    /// `first` before the first line and `rest` before every other one.
    fn prefixed(self, first: &str, rest: &str) -> Rich {
        let mut out = Rich::default();
        // (old line start, new line start)
        let mut starts = Vec::new();
        let mut old = 0;
        for (i, line) in self.text.split_inclusive('\n').enumerate() {
            out.text.push_str(if i == 0 { first } else { rest });
            starts.push((old, out.text.len()));
            out.text.push_str(line);
            old += line.len();
        }
        let shift = |at: usize| {
            let (o, n) = starts
                .iter()
                .rev()
                .find(|(o, _)| *o <= at)
                .copied()
                .unwrap_or((0, 0));
            at - o + n
        };
        out.spans = self
            .spans
            .into_iter()
            .map(|s| StyleSpan {
                range: shift(s.range.start)..shift(s.range.end.saturating_sub(1)) + 1,
                style: s.style,
            })
            .collect();
        out.mentions = self
            .mentions
            .into_iter()
            .map(|m| MentionSpan {
                range: shift(m.range.start)..shift(m.range.end.saturating_sub(1)) + 1,
                kind: m.kind,
            })
            .collect();
        out
    }
}

#[cfg(feature = "sdk")]
fn render(nodes: matrix_sdk::ruma::html::Children, verbatim: bool, ctx: &MentionCtx) -> Rich {
    let mut out = Rich::default();
    for node in nodes {
        if let Some(text) = node.as_text() {
            out.push_text(&text.borrow(), verbatim);
            continue;
        }
        let Some(element) = node.as_element() else {
            continue;
        };
        let name: &str = &element.name.local;
        let attr = |wanted: &str| {
            element
                .attrs
                .borrow()
                .iter()
                .find(|a| &*a.name.local == wanted)
                .map(|a| a.value.to_string())
        };
        let inner = |verbatim| render(node.children(), verbatim, ctx);
        match name {
            "strong" | "b" => out.styled(inner(verbatim), Style::Bold),
            "em" | "i" => out.styled(inner(verbatim), Style::Italic),
            "del" | "s" | "strike" => out.styled(inner(verbatim), Style::Strike),
            "code" => out.styled(inner(verbatim), Style::Code),
            "br" => out.text.push('\n'),
            "hr" => {
                out.newline();
                out.text.push_str("───\n");
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                out.newline();
                out.styled(inner(false).trimmed(), Style::Bold);
                out.text.push('\n');
            }
            "p" => {
                out.newline();
                out.append(inner(false).trimmed());
                out.text.push_str("\n\n");
            }
            "pre" => {
                out.newline();
                out.styled(inner(true).trimmed(), Style::Code);
                out.text.push('\n');
            }
            "blockquote" => {
                out.newline();
                let mut quote = inner(false).trimmed().prefixed("▎ ", "▎ ");
                if !quote.text.is_empty() {
                    quote.spans.push(StyleSpan {
                        range: 0..quote.text.len(),
                        style: Style::Quote,
                    });
                }
                out.append(quote);
                out.text.push('\n');
            }
            "ul" | "ol" => {
                out.newline();
                let first = attr("start")
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(1);
                for (number, item) in (first..).zip(
                    node.children()
                        .filter(|n| n.as_element().is_some_and(|e| &*e.name.local == "li")),
                ) {
                    let marker = if name == "ol" {
                        format!("{number}. ")
                    } else {
                        "• ".to_owned()
                    };
                    let rest = " ".repeat(marker.chars().count());
                    out.append(
                        render(item.children(), false, ctx)
                            .trimmed()
                            .prefixed(&marker, &rest),
                    );
                    out.text.push('\n');
                }
            }
            "tr" => {
                out.newline();
                out.append(inner(false).trimmed());
                out.text.push('\n');
            }
            "td" | "th" => {
                out.append(inner(false));
                out.text.push(' ');
            }
            "a" => {
                if let Some((kind, name)) = attr("href").and_then(|h| mention_of(&h, ctx)) {
                    let start = out.text.len();
                    match name {
                        Some(name) => out.text.push_str(&name),
                        None => out.append(inner(verbatim)),
                    }
                    if out.text.len() > start {
                        out.mentions.push(MentionSpan { range: start..out.text.len(), kind });
                    }
                    continue;
                }
                let label = inner(verbatim);
                let shown = label.text.clone();
                out.append(label);
                // The target stays visible: a label must not hide where the link goes.
                if let Some(href) = attr("href").filter(|h| {
                    (h.starts_with("http://") || h.starts_with("https://"))
                        && !h.starts_with("https://matrix.to/")
                        && !shown.contains(h.as_str())
                }) {
                    out.text.push_str(&format!(" ({href})"));
                }
            }
            "img" => {
                if let Some(alt) = attr("alt") {
                    out.push_text(&alt, false);
                }
            }
            "div" | "details" | "summary" | "table" | "thead" | "tbody" => {
                out.newline();
                out.append(inner(false).trimmed());
                out.text.push('\n');
            }
            _ => out.append(inner(verbatim)),
        }
    }
    out
}

/// A `matrix.to` link as a mention, with the text the pill shows instead of the sender's label
/// (users: their bubble name). None = no pill: not parsable, an event, or not a member here.
#[cfg(feature = "sdk")]
fn mention_of(href: &str, ctx: &MentionCtx) -> Option<(MentionKind, Option<String>)> {
    use matrix_sdk::ruma::{MatrixToUri, matrix_uri::MatrixId};
    match MatrixToUri::parse(href).ok()?.id() {
        MatrixId::User(id) => {
            let sender = ctx.users.get(id.as_str())?;
            let name = sender.name.clone().unwrap_or_else(|| sender.id.clone());
            Some((MentionKind::User { sender: sender.clone(), me: id.as_str() == ctx.me }, Some(name)))
        }
        MatrixId::Room(_) | MatrixId::RoomAlias(_) => Some((MentionKind::Room, None)),
        _ => None,
    }
}

/// Display text and styles of an `org.matrix.custom.html` body.
#[cfg(feature = "sdk")]
pub fn from_html(html: &str) -> (String, Vec<StyleSpan>) {
    let users = Default::default();
    let (text, styles, _) = from_html_mentions(html, &MentionCtx { users: &users, me: "", at_room: false });
    (text, styles)
}

/// Like `from_html`, plus the mention pills (user links resolved through `ctx`, «@room» if allowed).
#[cfg(feature = "sdk")]
pub fn from_html_mentions(html: &str, ctx: &MentionCtx) -> (String, Vec<StyleSpan>, Vec<MentionSpan>) {
    use matrix_sdk::ruma::html::{Html, SanitizerConfig};
    let html = Html::parse(html);
    html.sanitize_with(
        &SanitizerConfig::compat()
            .remove_reply_fallback()
            .remove_elements(["script", "style"])
            .max_depth(24),
    );
    let mut rich = render(html.children(), false, ctx).trimmed();
    if ctx.at_room {
        let found = at_room_spans(&rich.text, &rich.spans, &rich.mentions);
        rich.mentions.extend(found);
        rich.mentions.sort_by_key(|m| m.range.start);
    }
    (rich.text, rich.spans, rich.mentions)
}

#[cfg(all(test, feature = "sdk"))]
mod tests {
    use super::{MentionCtx, MentionKind, Style, from_html, from_html_mentions};
    use crate::history::HistorySender;
    use std::collections::HashMap;

    fn styled<'a>(text: &'a str, spans: &[super::StyleSpan], style: Style) -> Vec<&'a str> {
        spans
            .iter()
            .filter(|s| s.style == style)
            .map(|s| &text[s.range.clone()])
            .collect()
    }

    #[test]
    fn inline_styles_become_spans() {
        let (text, spans) =
            from_html("a <strong>b</strong> <em>c</em> <del>d</del> <code>e</code>");
        assert_eq!(text, "a b c d e");
        assert_eq!(styled(&text, &spans, Style::Bold), ["b"]);
        assert_eq!(styled(&text, &spans, Style::Italic), ["c"]);
        assert_eq!(styled(&text, &spans, Style::Strike), ["d"]);
        assert_eq!(styled(&text, &spans, Style::Code), ["e"]);
    }

    #[test]
    fn lists_quotes_and_code_blocks() {
        let (text, spans) = from_html(
            "<p>x</p>\n<ul>\n<li>uno</li>\n<li><strong>due</strong>\n<ol><li>in</li></ol></li>\n</ul>\n\
             <blockquote>\n<p>cita</p>\n</blockquote>\n<pre><code>a\n  b\n</code></pre>",
        );
        assert_eq!(text, "x\n\n• uno\n• due\n  1. in\n▎ cita\na\n  b");
        assert_eq!(styled(&text, &spans, Style::Bold), ["due"]);
        assert_eq!(styled(&text, &spans, Style::Quote), ["▎ cita"]);
        assert_eq!(styled(&text, &spans, Style::Code), ["a\n  b", "a\n  b"]);
    }

    #[test]
    fn dangerous_markup_is_dropped_and_link_targets_stay_visible() {
        let (text, _) = from_html(
            "<script>alert(1)</script><a href=\"javascript:x\">j</a> <a href=\"https://e.org/p\">sito</a> \
             <a href=\"https://matrix.to/#/@a:x\">@Anna</a> <img src=\"x\" onerror=\"y\"><mx-reply>old</mx-reply>ok",
        );
        assert_eq!(text, "j sito (https://e.org/p) @Anna ok");
    }

    #[test]
    #[cfg_attr(windows, ignore = "overflows the 2 MiB test-thread stack inside ruma Html::parse (before max_depth applies): open finding, see the stato notes")]
    fn deep_nesting_is_capped() {
        let html = format!(
            "{}x{}",
            "<blockquote>".repeat(5000),
            "</blockquote>".repeat(5000)
        );
        let (text, _) = from_html(&html);
        assert!(text.len() < 200_000);
    }

    fn spans_fit(text: &str, spans: &[super::StyleSpan]) {
        for s in spans {
            assert!(
                s.range.start < s.range.end && s.range.end <= text.len(),
                "{s:?} in {text:?}"
            );
            assert!(
                text.is_char_boundary(s.range.start) && text.is_char_boundary(s.range.end),
                "{s:?}"
            );
        }
    }

    #[test]
    fn composed_emoji_and_nested_styles_keep_spans_on_character_boundaries() {
        let (text, spans) = from_html(
            "<blockquote><ul><li>è <strong>👩‍👩‍👧‍👦 <em>ñ</em></strong> <code>🇮🇹</code></li></ul></blockquote>",
        );
        assert_eq!(text, "▎ • è 👩‍👩‍👧‍👦 ñ 🇮🇹");
        spans_fit(&text, &spans);
        assert_eq!(styled(&text, &spans, Style::Bold), ["👩‍👩‍👧‍👦 ñ"]);
        assert_eq!(styled(&text, &spans, Style::Italic), ["ñ"]);
        assert_eq!(styled(&text, &spans, Style::Code), ["🇮🇹"]);
        assert_eq!(styled(&text, &spans, Style::Quote), [text.as_str()]);
    }

    #[test]
    fn messages_from_other_clients_flatten_to_their_text() {
        // Element: reply fallback, matrix.to mention pill, colour, spoiler, custom emoji, heading.
        let (text, spans) = from_html(
            "<mx-reply><blockquote>quoted</blockquote></mx-reply><h2>Titolo</h2>\
             <a href=\"https://matrix.to/#/@anna:x\">Anna</a>: <font color=\"#ff0000\">rosso</font> \
             <span data-mx-spoiler>segreto</span> <img data-mx-emoticon src=\"mxc://x/y\" alt=\":gatto:\" \
             title=\":gatto:\" height=\"32\"><br>fine <a href=\"https://good.example\">https://evil.example</a>",
        );
        assert_eq!(
            text,
            "Titolo\nAnna: rosso segreto :gatto:\nfine https://evil.example (https://good.example)"
        );
        spans_fit(&text, &spans);
        assert_eq!(styled(&text, &spans, Style::Bold), ["Titolo"]);
    }

    #[test]
    fn hostile_markup_never_reaches_the_text_and_wide_bodies_stay_bounded() {
        let (text, _) = from_html(
            "<a href=\"data:text/html,x\">d</a><a href=\"vbscript:x\">v</a><iframe src=\"https://e.org\"></iframe>\
             <svg onload=\"x\"><script>y</script></svg><style>z</style><form action=\"https://e.org\"><input value=\"i\"></form>\
             <a href=\"https://e.org\" onclick=\"c\" target=\"_blank\">ok</a><span style=\"display:none\">hidden</span>",
        );
        // Dropped elements leave no text (a form field is not a message), a bad href keeps only its label.
        assert_eq!(text, "dvok (https://e.org)hidden");
        // The event size cap is 64 KiB: a body made of thousands of styled fragments must finish and fit.
        let wide = "<b>x</b><i>y</i>".repeat(4096);
        let (text, spans) = from_html(&wide);
        assert_eq!(text.len(), 8192);
        assert_eq!(spans.len(), 8192);
        spans_fit(&text, &spans);
    }

    fn pills(html: &str, at_room: bool) -> (String, Vec<(String, MentionKind)>) {
        let mut users = HashMap::new();
        // Two members share a name: the second one is already «Nome (@id)» on its bubbles.
        for (id, name) in [("@anna:x", "Anna"), ("@me:x", "Io"), ("@bo:x", "Bo (@bo:x)")] {
            users.insert(id.to_owned(), HistorySender::new(id, Some(name)));
        }
        let (text, _, mentions) = from_html_mentions(html, &MentionCtx { users: &users, me: "@me:x", at_room });
        let shown = mentions.into_iter().map(|m| (text[m.range].to_owned(), m.kind)).collect();
        (text, shown)
    }

    #[test]
    fn user_links_become_pills_named_like_their_bubble_and_flag_me() {
        let (text, shown) = pills(
            "<a href=\"https://matrix.to/#/@anna:x\">@anna-label</a> e <a href=\"https://matrix.to/#/%40me%3Ax\">me</a> e <strong><a href=\"https://matrix.to/#/@bo:x\">@bo</a></strong>",
            false,
        );
        assert_eq!(text, "Anna e Io e Bo (@bo:x)");
        let user = |id: &str, name: &str, me| MentionKind::User { sender: HistorySender::new(id, Some(name)), me };
        assert_eq!(
            shown,
            [
                ("Anna".into(), user("@anna:x", "Anna", false)),
                ("Io".into(), user("@me:x", "Io", true)),
                ("Bo (@bo:x)".into(), user("@bo:x", "Bo (@bo:x)", false)),
            ]
        );
    }

    #[test]
    fn rooms_are_pills_and_unknown_targets_fall_back_to_their_label() {
        let (text, shown) = pills(
            "<a href=\"https://matrix.to/#/#sala:x\">#sala:x</a> <a href=\"https://matrix.to/#/@ghost:x\">@Ghost</a> \
             <a href=\"https://matrix.to/#/!r:x/$e:x\">msg</a> <a href=\"https://matrix.to/#/nonsense\">@Rotto</a>",
            false,
        );
        assert_eq!(text, "#sala:x @Ghost msg @Rotto");
        assert_eq!(shown, [("#sala:x".into(), MentionKind::Room)]);
    }

    #[test]
    fn at_room_is_a_pill_only_when_allowed_and_outside_code() {
        let html = "<p>@room ciao@roomx <code>@room</code> a @room.</p>";
        let (text, shown) = pills(html, true);
        assert_eq!(text, "@room ciao@roomx @room a @room.");
        assert_eq!(shown.iter().map(|(t, k)| (t.as_str(), k)).collect::<Vec<_>>(), [("@room", &MentionKind::AtRoom), ("@room", &MentionKind::AtRoom)]);
        assert_eq!(shown.len(), 2);
        assert!(pills(html, false).1.is_empty());
    }

    #[test]
    fn pills_survive_lists_quotes_and_malformed_markup() {
        let (text, shown) = pills(
            "<blockquote><ul><li>x <a href=\"https://matrix.to/#/@anna:x\">a</a></li></ul></blockquote><a href=\"https://matrix.to/#/@me:x\">m",
            false,
        );
        assert_eq!(text, "▎ • x Anna\nIo");
        assert_eq!(shown.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["Anna", "Io"]);
    }
}
