//! Name line of a chat-list row: the name takes what is left and ends with «…», the stamp is never shortened.
use gpui::{Div, ParentElement, StyledText, Styled, div, px, rgb};

pub(crate) fn name_line(name: StyledText, stamp: Option<StyledText>, stamp_color: u32) -> Div {
    use matrix_desktop::row_text::{NAME_LINE_PX, NAME_PX};
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(NAME_LINE_PX))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(NAME_PX))
                .line_height(px(NAME_LINE_PX))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(name),
        )
        .children(stamp.map(|stamp| {
            div().flex_shrink_0().text_size(px(11.)).line_height(px(NAME_LINE_PX)).text_color(rgb(stamp_color)).child(stamp)
        }))
}

/// Painted check on a private display: `the ct script 'the gui-test-x11 script "live" long_name_ends_with_ellipsis_and_full_stamp_stays'`.
#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, Context, IntoElement, Render, TextLayout, Window};

    const STAMP: &str = "25/09/26";
    const ROW_WIDTH_PX: f32 = 240.;
    type Seen = (String, String, f32, f32, f32);
    static SEEN: std::sync::Mutex<Option<Seen>> = std::sync::Mutex::new(None);

    const LONG: &str = "Gruppo coordinamento progetti e forniture di lunghezza esagerata";
    // Layouts of the last render (every render builds new text elements).
    thread_local!(static LAYOUTS: std::cell::RefCell<Option<(TextLayout, TextLayout)>> = const { std::cell::RefCell::new(None) });

    struct Host;
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let (name, stamp) = (StyledText::new(LONG), StyledText::new(STAMP));
            LAYOUTS.with(|l| *l.borrow_mut() = Some((name.layout().clone(), stamp.layout().clone())));
            div().w(px(ROW_WIDTH_PX)).child(name_line(name, Some(stamp), 0x666666))
        }
    }

    #[test]
    #[ignore = "requires private display"]
    fn long_name_ends_with_ellipsis_and_full_stamp_stays() {
        assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
        gpui::Application::new().run(|cx| {
            cx.open_window(gpui::WindowOptions::default(), |_, cx| cx.new(|_| Host)).unwrap();
            let ex = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                ex.timer(std::time::Duration::from_millis(400)).await;
                let (name_layout, stamp_layout) = LAYOUTS.with(|l| l.borrow().clone()).expect("never rendered");
                let (nb, sb) = (name_layout.bounds(), stamp_layout.bounds());
                // Natural width of the stamp: its text alone, no truncation.
                *SEEN.lock().unwrap() = Some((name_layout.text(), stamp_layout.text(), f32::from(nb.right()), f32::from(sb.left()), f32::from(sb.size.width)));
                eprintln!("name {:?} bounds {nb:?}; stamp bounds {sb:?}", name_layout.text());
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
        let (name, stamp, name_right, stamp_left, stamp_w) = SEEN.lock().unwrap().take().expect("never painted");
        assert_eq!(stamp, STAMP, "stamp text was shortened");
        assert!(name.ends_with('…'), "long name must end with «…», got {name:?}");
        assert!(name_right <= stamp_left + 0.5, "name ({name_right}) overlaps the stamp ({stamp_left})");
        assert!(stamp_left + stamp_w <= ROW_WIDTH_PX + 0.5, "stamp leaves the row: {stamp_left}+{stamp_w}");
        eprintln!("STAMP COLUMN WIDTH {stamp_w} px");
    }
}
