//! «@name» suggestions above the composer: members matching what is typed after the `@` before the cursor.
//! Up/Down move the highlight, Enter or Tab (or a click) put «@Name » in the text and remember who it is, so the send
//! turns it into a real mention; Esc closes. The keys are taken before the text box sees them.
use eframe::egui::{self, Key, Modifiers};
use matrix_engine::operations::Member;

const WIDTH: f32 = 320.;

struct Open {
    at: usize,
    cursor: usize,
    matches: Vec<Member>,
    sel: usize,
    key: String,
}

fn open(ui: &egui::Ui, id: egui::Id, draft: &str, members: &[Member]) -> Option<Open> {
    // A press on a row moves the text focus to the row: the list stays while the pointer is over it, until the click lands.
    let over = ui.data(|d| d.get_temp::<egui::Rect>(id.with("rect"))).is_some_and(|r| ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| r.contains(p)));
    // A window the system never focused (headless seat) keeps no text focus: the list then follows the text alone.
    if members.is_empty() || !(ui.memory(|m| m.has_focus(id)) || over || !ui.input(|i| i.focused)) {
        return None;
    }
    // No cursor yet (text set from outside, nothing clicked): the end of the text.
    let cursor = egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range()).map_or(draft.len(), |r| draft.char_indices().nth(r.primary.index.into()).map_or(draft.len(), |(b, _)| b));
    let Some((at, query)) = matrix_desktop::mention_token(draft, cursor) else {
        ui.data_mut(|d| d.remove::<usize>(id.with("off"))); // a closed list stays closed only until the `@` is gone
        return None;
    };
    if ui.data(|d| d.get_temp::<usize>(id.with("off"))) == Some(at) {
        return None;
    }
    let matches: Vec<Member> = matrix_desktop::mention_matches(members, query).into_iter().cloned().collect();
    if matches.is_empty() {
        return None;
    }
    let key = format!("{at}:{query}");
    let sel = ui.data(|d| d.get_temp::<(String, usize)>(id.with("sel"))).filter(|(k, _)| *k == key).map_or(0, |(_, s)| s.min(matches.len() - 1));
    Some(Open { at, cursor, matches, sel, key })
}

fn insert(ui: &egui::Ui, id: egui::Id, draft: &mut String, mentions: &mut Vec<matrix_engine::sending::Mention>, o: &Open, m: &Member) {
    let label = format!("@{}", m.name.trim_start_matches('@'));
    draft.replace_range(o.at..o.cursor, &format!("{label} "));
    mentions.push(matrix_engine::sending::Mention { user_id: m.user_id.clone(), label: label.clone() });
    let end = draft[..o.at].chars().count() + label.chars().count() + 1;
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
    state.store(ui.ctx(), id);
    ui.ctx().memory_mut(|m| m.request_focus(id));
}

/// Before the text box: Up/Down/Enter/Tab/Esc belong to the open list.
pub fn keys(ui: &egui::Ui, id: egui::Id, draft: &mut String, members: &[Member], mentions: &mut Vec<matrix_engine::sending::Mention>) {
    let Some(o) = open(ui, id, draft, members) else { return };
    let (down, up, pick, esc) = ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::ArrowDown), i.consume_key(Modifiers::NONE, Key::ArrowUp), i.consume_key(Modifiers::NONE, Key::Enter) | i.consume_key(Modifiers::NONE, Key::Tab), i.consume_key(Modifiers::NONE, Key::Escape)));
    if esc {
        ui.data_mut(|d| d.insert_temp(id.with("off"), o.at));
    } else if pick {
        insert(ui, id, draft, mentions, &o, &o.matches[o.sel]);
    } else if down || up {
        let sel = if down { (o.sel + 1).min(o.matches.len() - 1) } else { o.sel.saturating_sub(1) };
        ui.data_mut(|d| d.insert_temp(id.with("sel"), (o.key, sel)));
    }
}

/// After the text box: the list hangs above `field`; hover moves the highlight, a click inserts.
pub fn show(ui: &mut egui::Ui, id: egui::Id, field: egui::Rect, draft: &mut String, members: &[Member], mentions: &mut Vec<matrix_engine::sending::Mention>, avatars: Option<&crate::avatar::Avatars>, t: &matrix_desktop::DesignTokens) {
    let Some(o) = open(ui, id, draft, members) else {
        ui.data_mut(|d| d.remove::<egui::Rect>(id.with("rect")));
        return;
    };
    let mxc = |m: &Member| m.avatar.clone().filter(|a| !a.is_empty());
    // Only the rows on screen ask for their picture.
    ui.data_mut(|d| d.insert_temp(egui::Id::new(crate::avatar::MENTION_AVATARS), o.matches.iter().filter_map(mxc).collect::<Vec<_>>()));
    let duplicates = crate::rows::duplicate_names(o.matches.iter().map(|m| m.name.as_str()));
    let (mut pick, mut hover) = (None, None);
    let safe = ui.ctx().content_rect();
    let w = WIDTH.min(safe.width() - 8.);
    let x = if crate::dir::rtl() { field.right().clamp(safe.left() + w + 4., safe.right() - 4.) } else { field.left().clamp(safe.left() + 4., safe.right() - w - 4.) };
    let moved = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    let area = egui::Area::new(id.with("mentions")).order(egui::Order::Foreground).pivot(crate::dir::align(egui::Align2::LEFT_BOTTOM)).fixed_pos(egui::pos2(x, field.top() - 8.)).show(ui.ctx(), |ui| {
        let frame = egui::Frame::popup(ui.style());
        frame.show(ui, |ui| {
            ui.set_width(w - frame.total_margin().left - frame.total_margin().right);
            for (i, (m, duplicate)) in o.matches.iter().zip(duplicates).enumerate() {
                let picture = avatars.map_or(crate::avatar::Avatar::Missing, |a| crate::avatar::lookup(a, mxc(m).as_deref()));
                let r = ui.push_id(&m.user_id, |ui| crate::rows::person_row(ui, &m.name, &m.user_id, duplicate, i == o.sel, &picture, t)).inner;
                if moved && r.hovered() { hover = Some(i); }
                if r.clicked() { pick = Some(i); }
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(id.with("rect"), area.response.rect));
    if let Some(i) = hover.filter(|i| *i != o.sel) {
        ui.data_mut(|d| d.insert_temp(id.with("sel"), (o.key.clone(), i)));
        ui.ctx().request_repaint();
    }
    if let Some(i) = pick {
        insert(ui, id, draft, mentions, &o, &o.matches[i]);
    }
}

/// Debug builds, GNOBBI_EGUI_VIEW=mention: the composer with «@Mar» typed and the list open, on the sign-in screen, for screenshots
/// without an account; GNOBBI_EGUI_PHOTO=<png>[:<png>] are the photos of the first two entries.
#[cfg(debug_assertions)]
pub fn demo(ctx: &egui::Context, t: &matrix_desktop::DesignTokens) {
    type State = (String, Vec<matrix_engine::sending::Mention>, crate::avatar::Avatars, crate::stickers::Panel);
    thread_local!(static DEMO: std::cell::RefCell<State> = std::cell::RefCell::new(("@Mar".into(), vec![], Default::default(), Default::default())));
    if matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW").as_deref() != Ok("mention") {
        return;
    }
    let photos = matrix_desktop::hooks::var("GNOBBI_EGUI_PHOTO").unwrap_or_default();
    let who = |id: &str, name: &str, n: usize| Member { user_id: id.into(), name: name.into(), avatar: photos.split(':').nth(n).map(|_| format!("mxc://demo/{n}")), role: matrix_domain::role::MemberRole::Member };
    let members = [who("@meska:gnobbi.chat", "Marco", 0), who("@meska:mecomsrl.com", "Marco", 1), who("@marcella:gnobbi.chat", "Marcella", 9)];
    DEMO.with_borrow_mut(|(draft, mentions, avatars, panel)| {
        if avatars.is_empty() {
            for (n, path) in photos.split(':').enumerate().filter(|(_, p)| !p.is_empty()) {
                let image = matrix_engine::profile::decode_blocking(std::path::Path::new(path)).map(|p| p.into_rgba8()).expect("demo photo");
                let size = [image.width() as usize, image.height() as usize];
                avatars.insert(format!("mxc://demo/{n}"), crate::avatar::Avatar::Ready(ctx.load_texture(path, egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()), Default::default())));
            }
        }
        egui::Area::new(egui::Id::new("mention-demo")).anchor(egui::Align2::LEFT_BOTTOM, [0., 0.]).show(ctx, |ui| {
            ui.set_width(ctx.content_rect().width());
            let (mut a, mut b, mut c, mut d, mut e, mut f) = (false, false, false, None, None, None);
            crate::composer::show(ui, "", draft, crate::composer::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &members, mentions, Some((panel, avatars)), t);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2};

    fn member(id: &str, name: &str, avatar: Option<&str>) -> Member {
        Member { user_id: id.into(), name: name.into(), avatar: avatar.map(Into::into), role: matrix_domain::role::MemberRole::Member }
    }

    fn members() -> Vec<Member> {
        vec![member("@a:x", "Marco", Some("mxc://x/a")), member("@b:y", "Marco", None), member("@c:x", "Marcella", None)]
    }

    fn key(key: Key) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    /// Composer with the draft «@Mar» focused; `steps` are the events of the frames after the list opened.
    struct Bar { ctx: egui::Context, draft: String, mentions: Vec<matrix_engine::sending::Mention>, sent: Option<String>, avatars: crate::avatar::Avatars, panel: crate::stickers::Panel, size: egui::Vec2 }

    impl Bar {
        fn new(size: egui::Vec2) -> Self {
            let mut bar = Bar { ctx: crate::icons::test_context(), draft: "@Mar".into(), mentions: vec![], sent: None, avatars: Default::default(), panel: Default::default(), size };
            for frame in 0..4 {
                if frame == 2 { bar.ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-text"))); }
                bar.frame(vec![]);
            }
            bar
        }
        fn frame(&mut self, events: Vec<Event>) {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)), events, ..Default::default() };
            let (mut a, mut b, mut c, mut d, mut e, mut f) = (false, false, false, None, None, None);
            let mut out = self.ctx.run_ui(input, |ui| {
                crate::composer::panel(egui::Frame::NONE).show(ui, |ui| {
                    let sent = crate::composer::show(ui, "", &mut self.draft, crate::composer::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &members(), &mut self.mentions, Some((&mut self.panel, &self.avatars)), &matrix_desktop::DARK);
                    self.sent = self.sent.take().or(sent);
                });
            });
            out.textures_delta.clear();
        }
        fn popup(&self) -> Option<egui::Rect> {
            self.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("composer-text").with("rect")))
        }
    }

    const PHONE: egui::Vec2 = egui::vec2(436., 800.);

    #[test]
    fn popup_is_above_the_text_box_and_inside_the_window() {
        for size in [egui::vec2(900., 600.), PHONE] {
            let bar = Bar::new(size);
            let popup = bar.popup().expect("popup open");
            let field = bar.ctx.read_response(egui::Id::new("composer-text")).unwrap().rect;
            assert!(!popup.intersects(field), "popup {popup:?} covers the text {field:?}");
            assert!(popup.bottom() <= field.top() && popup.left() >= 0. && popup.right() <= size.x && popup.top() >= 0., "{popup:?} {field:?}");
        }
    }

    #[test]
    fn arrows_move_the_highlight_and_enter_inserts_without_sending() {
        let mut bar = Bar::new(PHONE);
        bar.frame(vec![key(Key::ArrowDown)]);
        bar.frame(vec![key(Key::Enter)]);
        assert_eq!(bar.sent, None, "Enter picks, it does not send");
        assert_eq!(bar.draft, "@Marco ");
        assert_eq!(bar.mentions.len(), 1);
        assert_eq!(bar.mentions[0].user_id, "@a:x", "one Down = second entry (Marcella sorts first)");
        let mut bar = Bar::new(PHONE);
        bar.frame(vec![key(Key::ArrowDown)]);
        bar.frame(vec![key(Key::ArrowDown)]);
        bar.frame(vec![key(Key::Tab)]);
        assert_eq!(bar.mentions[0].user_id, "@b:y", "two Downs = third, Tab inserts too");
        let mut bar = Bar::new(PHONE);
        bar.frame(vec![key(Key::Enter)]);
        assert_eq!(bar.mentions[0].user_id, "@c:x", "first entry is highlighted by default");
        bar.frame(vec![]);
        assert!(bar.popup().is_none(), "the list closes once the mention is in");
    }

    #[test]
    fn escape_closes_without_inserting_and_enter_then_sends() {
        let mut bar = Bar::new(PHONE);
        bar.frame(vec![key(Key::Escape)]);
        bar.frame(vec![]);
        assert!(bar.popup().is_none() && bar.draft == "@Mar" && bar.mentions.is_empty());
        bar.frame(vec![key(Key::Enter)]);
        assert_eq!(bar.sent.as_deref(), Some("@Mar"));
    }

    #[test]
    fn click_on_a_row_inserts_it() {
        let mut bar = Bar::new(egui::vec2(900., 600.));
        let popup = bar.popup().unwrap();
        let pos = popup.left_top() + egui::vec2(60., 70.); // second row: 6 px frame, 44 px rows
        let button = |pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        bar.frame(vec![Event::PointerMoved(pos)]);
        bar.frame(vec![button(true)]);
        bar.frame(vec![button(false)]);
        bar.frame(vec![]);
        assert_eq!(bar.mentions.len(), 1, "draft {:?}", bar.draft);
        assert_eq!(bar.mentions[0].user_id, "@a:x");
        assert!(bar.draft.ends_with(' '), "{:?}", bar.draft);
    }

    #[test]
    fn visible_rows_ask_for_their_picture_and_show_it_instead_of_initials() {
        let mut bar = Bar::new(PHONE);
        let wanted = bar.ctx.data(|d| d.get_temp::<Vec<String>>(egui::Id::new(crate::avatar::MENTION_AVATARS)));
        assert_eq!(wanted, Some(vec!["mxc://x/a".to_owned()]), "only the visible entries with a photo");
        let tex = bar.ctx.load_texture("a", egui::ColorImage::example(), Default::default());
        bar.avatars.insert("mxc://x/a".into(), crate::avatar::Avatar::Ready(tex));
        let ctx = bar.ctx.clone();
        let mut shapes = vec![];
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, PHONE)), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            let r = crate::rows::person_row(ui, "Marco", "@a:x", false, false, &crate::avatar::lookup(&bar.avatars, Some("mxc://x/a")), &matrix_desktop::DARK);
            assert!(r.rect.height() > 0.);
        });
        out.textures_delta.clear();
        fn flat(s: &egui::Shape, into: &mut Vec<egui::Shape>) {
            if let egui::Shape::Vec(v) = s { v.iter().for_each(|s| flat(s, into)); } else { into.push(s.clone()); }
        }
        for s in &out.shapes { flat(&s.shape, &mut shapes); }
        assert!(shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.brush.is_some()) || matches!(s, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default())), "the photo is drawn");
        assert!(!shapes.iter().any(|s| matches!(s, egui::Shape::Text(t) if t.galley.text() == "MA" || t.galley.text() == "Ma")), "no initials over a photo");
    }
}
