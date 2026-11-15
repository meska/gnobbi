//! Event routing of the wheel under an overlay: the shared `dialog_shell` consumes it (no scroll chaining, short content too);
//! outside the overlay the chat still scrolls. Plus a guard that every full-window `*-layer` overlay occludes the layer below.
use crate::*;
use gpui::{AppContext as _, ListAlignment, ListState, Pixels, Render, ScrollDelta, ScrollHandle, ScrollWheelEvent, StatefulInteractiveElement, TestAppContext, list, point, size};

struct View { chat: ListState, popup: ScrollHandle, content: f32 }
impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let dialog = div().id("popup").h(px(100.)).child(
            div().id("plist").size_full().overflow_y_scroll().track_scroll(&self.popup).child(div().h(px(self.content)).w_full()),
        );
        div().size_full().relative()
            .child(list(self.chat.clone(), |_, _, _| div().h(px(20.)).w_full().into_any_element()).size_full())
            .child(crate::menu_view::dialog_shell("layer", dialog, 100., matrix_desktop::LIGHT, |_, _, _| {}))
    }
}

/// Wheel down 50 px at (x, 150) → (chat offset y, popup offset y). Window 300x300, dialog centred (x 100..200).
fn wheel(cx: &mut TestAppContext, x: f32, content: f32, popup_offset: f32) -> (Pixels, Pixels) {
    let cx = cx.add_empty_window();
    let chat = ListState::new(100, ListAlignment::Top, px(20.));
    let popup = ScrollHandle::new();
    let (c, p) = (chat.clone(), popup.clone());
    cx.draw(point(px(0.), px(0.)), size(px(300.), px(300.)), move |_, cx| cx.new(|_| View { chat: c, popup: p, content }));
    popup.set_offset(point(px(0.), px(popup_offset)));
    cx.simulate_event(ScrollWheelEvent { position: point(px(x), px(150.)), delta: ScrollDelta::Pixels(point(px(0.), px(-50.))), ..Default::default() });
    (chat.scroll_px_offset_for_scrollbar().y, popup.offset().y)
}

#[gpui::test]
fn wheel_over_overlay_scrolls_it_only(cx: &mut TestAppContext) {
    let (chat, popup) = wheel(cx, 150., 400., 0.);
    assert_eq!((chat, popup), (px(0.), px(-50.)));
}
#[gpui::test]
fn wheel_at_end_of_overlay_does_not_chain(cx: &mut TestAppContext) {
    assert_eq!(wheel(cx, 150., 400., -300.).0, px(0.));
}
#[gpui::test]
fn wheel_over_short_overlay_does_not_scroll_chat(cx: &mut TestAppContext) {
    assert_eq!(wheel(cx, 150., 50., 0.).0, px(0.));
}
#[gpui::test]
fn wheel_outside_overlay_scrolls_chat(cx: &mut TestAppContext) {
    // Over the scrim (x = 20): the scrim occludes too, so the chat stays put under any overlay; the chat scrolls only with no overlay.
    let cx = cx.add_empty_window();
    let chat = ListState::new(100, ListAlignment::Top, px(20.));
    let c = chat.clone();
    cx.draw(point(px(0.), px(0.)), size(px(300.), px(300.)), move |_, cx| {
        cx.new(|_| NoOverlay(c))
    });
    cx.simulate_event(ScrollWheelEvent { position: point(px(20.), px(150.)), delta: ScrollDelta::Pixels(point(px(0.), px(-50.))), ..Default::default() });
    assert_ne!(chat.scroll_px_offset_for_scrollbar().y, px(0.));
}
struct NoOverlay(ListState);
impl Render for NoOverlay {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(list(self.0.clone(), |_, _, _| div().h(px(20.)).w_full().into_any_element()).size_full())
    }
}

/// Every `.id("...-layer")` overlay in the views must occlude (own the wheel) right after its id.
#[test]
fn every_overlay_layer_occludes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut missing = vec![];
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "rs") || p.file_name().unwrap() == "overlay_scroll_test.rs" { continue; }
        let s = std::fs::read_to_string(&p).unwrap();
        let lines: Vec<_> = s.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if l.trim().starts_with(".id(\"") && l.contains("-layer\")") && !lines.get(i + 1).is_some_and(|n| n.trim() == ".occlude()") {
                missing.push(format!("{}:{}", p.display(), i + 1));
            }
        }
    }
    assert!(missing.is_empty(), "overlay layers without .occlude(): {missing:?}");
}
