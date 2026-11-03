//! Bundled Noto Color Emoji (SIL OFL 1.1): one emoji set on every platform.
//! Linux only: gpui's Linux text system draws every emoji cluster with it (vendor/gpui-0.2.2/GNOBBI-PATCH.md).
//! macOS keeps Apple Color Emoji (CoreText cannot draw CBDT/COLRv1 fonts); Windows keeps Segoe UI Emoji.

#[cfg(target_os = "linux")]
static NOTO_COLOR_EMOJI: &[u8] = include_bytes!("../assets/fonts/NotoColorEmoji.ttf");

/// Registers the font; call once before the first window.
pub fn register(cx: &gpui::App) {
    #[cfg(target_os = "linux")]
    cx.text_system()
        .add_fonts(vec![std::borrow::Cow::Borrowed(NOTO_COLOR_EMOJI)])
        .expect("register bundled emoji font");
    #[cfg(not(target_os = "linux"))]
    let _ = cx;
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use gpui::{AppContext as _, Font, FontStyle, FontWeight, TextRun, px};

    fn shape(window: &gpui::Window, text: &str) -> gpui::ShapedLine {
        let run = TextRun {
            len: text.len(),
            font: Font {
                family: matrix_desktop::UI_FONT.into(),
                features: Default::default(),
                fallbacks: None,
                weight: FontWeight::NORMAL,
                style: FontStyle::Normal,
            },
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text.to_string().into(), px(16.), &[run], None)
    }

    /// Fails if the embedded file is missing/corrupt (no emoji glyph) or a text font wins over Noto.
    #[test]
    #[ignore = "requires private display"]
    fn every_emoji_cluster_is_one_noto_glyph() {
        gpui::Application::new().run(|cx| {
            super::register(cx);
            cx.open_window(gpui::WindowOptions::default(), |window, cx| {
                let size = shape(window, "😀").width;
                assert!(size > px(8.), "emoji width {size:?}");
                for seq in [
                    "😀",
                    "😁",
                    "😂",
                    "🤣",
                    "☕",
                    "✅",
                    "❌",
                    "✨",
                    "👍",
                    "👨‍👩‍👧",
                    "👍🏽",
                    "🇮🇹",
                    "1️⃣",
                    "❤️",
                ] {
                    let line = shape(window, seq);
                    let glyphs: Vec<_> = line.runs.iter().flat_map(|r| &r.glyphs).collect();
                    assert_eq!(glyphs.len(), 1, "{seq}: one glyph per cluster");
                    assert!(glyphs[0].is_emoji, "{seq}: drawn with Noto Color Emoji");
                    assert_eq!(line.runs.len(), 1, "{seq}: one run");
                    assert!(
                        (line.width - size).abs() < px(0.5),
                        "{seq}: width {:?} vs {size:?}",
                        line.width
                    );
                }
                // Text around an emoji keeps its own font; "❤" without VS16 stays text.
                let mixed = shape(window, "ciao 😀 ciao");
                assert_eq!(mixed.runs.len(), 3);
                assert!(!shape(window, "❤").runs[0].glyphs[0].is_emoji);
                cx.new(|_| gpui::Empty)
            })
            .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(200)).await;
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }
}
