//! Formatted message bodies (`org.matrix.custom.html`) flattened to display text plus style spans,
//! so the timeline draws them with its plain styled text. The HTML is sanitized first (SDK/ruma
//! compat rules, reply fallback removed, nesting capped).
// shortcut: no inline images, tables are flattened to lines, a labelled link shows its address in
// brackets after the label; upgrade when the timeline draws blocks instead of one styled text.
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

#[cfg(feature = "sdk")]
#[derive(Default)]
struct Rich {
    text: String,
    spans: Vec<StyleSpan>,
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
        out
    }
}

#[cfg(feature = "sdk")]
fn render(nodes: matrix_sdk::ruma::html::Children, verbatim: bool) -> Rich {
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
        let inner = |verbatim| render(node.children(), verbatim);
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
                        render(item.children(), false)
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

/// Display text and styles of an `org.matrix.custom.html` body.
#[cfg(feature = "sdk")]
pub fn from_html(html: &str) -> (String, Vec<StyleSpan>) {
    use matrix_sdk::ruma::html::{Html, SanitizerConfig};
    let html = Html::parse(html);
    html.sanitize_with(
        &SanitizerConfig::compat()
            .remove_reply_fallback()
            .remove_elements(["script", "style"])
            .max_depth(24),
    );
    let rich = render(html.children(), false).trimmed();
    (rich.text, rich.spans)
}

#[cfg(all(test, feature = "sdk"))]
mod tests {
    use super::{Style, from_html};

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
}
