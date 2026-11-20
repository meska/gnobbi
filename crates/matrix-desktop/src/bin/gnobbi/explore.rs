//! «Esplora stanze pubbliche»: the public-room directory of the account's server (or another one) as a window.
//! The search, debounce, paging and errors are the engine's (`SearchRooms` / `MoreRooms`); this file is the form and the rows.
use matrix_desktop::i18n;
use crate::{join::JoinState, list::{one_line, rgb}, session::Session};
use eframe::egui::{self, Align2, Sense, Vec2};
use matrix_desktop::{DesignTokens, add_server::normalize_server, quick_search::{Category, DirectoryRequest, room_request}, search_list::{Hit, Status, more_text}};
use matrix_engine::{operations::DirectoryRoom, search::{More, SearchState}};

const ROW: f32 = 76.;

fn button_color(t: &DesignTokens, accent_percent: u32) -> egui::Color32 {
    let channel = |shift: u32| ((((t.selected >> shift) & 255u32) * (100 - accent_percent) + ((t.accent >> shift) & 255u32) * accent_percent) / 100) as u8;
    egui::Color32::from_rgb(channel(16), channel(8), channel(0))
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Pick {
    /// The account's own server («Il tuo server»).
    #[default]
    Own,
    MatrixOrg,
    Other,
}

#[derive(Default)]
pub struct Explore {
    term: String,
    pick: Pick,
    other: String,
    spaces: bool,
    fresh: bool,
}

/// What a row offers: the room is ours, a join runs, or the user may start (or retry) one.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Open,
    Join,
    Joining,
    Retry(&'static str),
}

fn action(joined: bool, join: Option<&JoinState>) -> Action {
    match join {
        _ if joined => Action::Open,
        Some(JoinState::Joining | JoinState::StillJoining | JoinState::Joined(_)) => Action::Joining,
        Some(JoinState::Failed(c)) => Action::Retry(c.line()),
        _ => Action::Join,
    }
}

impl Explore {
    pub fn new() -> Self {
        Self { fresh: true, ..Default::default() }
    }

    pub fn set_term(&mut self, term: &str) {
        self.term = term.to_owned();
    }

    /// `Ok(None)` = the account's own server; `Err` = the typed name is not a server name.
    fn server(&self) -> Result<Option<String>, matrix_desktop::add_server::NameError> {
        match self.pick {
            Pick::Own => Ok(None),
            Pick::MatrixOrg => Ok(Some("matrix.org".into())),
            Pick::Other => normalize_server(&self.other).map(Some),
        }
    }

    /// What the engine is asked now: an empty or too short term is the server's first page («Suggerimenti»).
    pub fn request(&self) -> Option<DirectoryRequest> {
        room_request(Some(if self.spaces { Category::Spaces } else { Category::Rooms }), &self.term, self.server().ok()?)
    }
}

pub fn show(ctx: &egui::Context, s: &mut Session, t: &DesignTokens) {
    let Some(mut e) = s.explore.take() else { return };
    let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let request = e.request();
    let state = match (&request, &s.public_rooms) {
        (Some((term, ..)), st) if st.term() == Some(term.as_str()) => st.clone(),
        _ => SearchState::Idle,
    };
    let mut click: Option<(DirectoryRoom, bool)> = None;
    let mut more = false;
    egui::Window::new(i18n::t("Explore public rooms")).frame(egui::Frame::window(&ctx.style_of(ctx.theme())).stroke(egui::Stroke::NONE)).open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_width(460.);
        let field = ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut e.term).hint_text(i18n::t("Search public rooms")).align(Align2::LEFT_CENTER));
        if std::mem::take(&mut e.fresh) {
            field.request_focus();
        }
        ui.add_space(12.);
        ui.horizontal(|ui| {
            let label = match e.pick {
                Pick::Own => i18n::t("Your server"),
                Pick::MatrixOrg => "matrix.org",
                Pick::Other => i18n::t("Other server…"),
            };
            let combo = egui::ComboBox::from_id_salt("explore-server").selected_text(label).show_ui(ui, |ui| {
                ui.selectable_value(&mut e.pick, Pick::Own, i18n::t("Your server"));
                ui.selectable_value(&mut e.pick, Pick::MatrixOrg, "matrix.org");
                ui.selectable_value(&mut e.pick, Pick::Other, i18n::t("Other server…"));
            });
            crate::metrics::clickable(combo.response);
            if e.pick == Pick::Other {
                ui.add(crate::metrics::line(&mut e.other).hint_text(i18n::t("server name")).desired_width(150.));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.visuals_mut().selection.bg_fill = rgb(t.selection);
                ui.visuals_mut().selection.stroke = egui::Stroke::new(1., rgb(t.text));
                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
                ui.selectable_value(&mut e.spaces, true, i18n::t("Spaces"));
                ui.selectable_value(&mut e.spaces, false, i18n::t("Rooms"));
            });
        });
        if e.pick == Pick::Other && !e.other.trim().is_empty() && let Err(err) = e.server() {
            ui.colored_label(rgb(t.error), err.text());
        }
        ui.add_space(12.);
        let line = |ui: &mut egui::Ui, text: &str, color: u32| {
            ui.label(egui::RichText::new(text).color(rgb(color)));
        };
        match &state {
            _ if request.is_none() => {}
            SearchState::Idle | SearchState::Searching { .. } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    line(ui, Status::Searching.text(), t.secondary);
                });
            }
            SearchState::NoResults { .. } => line(ui, Status::NoResults.text(), t.secondary),
            SearchState::Error { error, .. } => line(ui, Status::Error(*error).text(), t.error),
            SearchState::Results { hits, more: m, .. } => {
                egui::ScrollArea::vertical().max_height(380.).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.;
                    for room in hits {
                        let joined = s.sidebar.rooms().iter().any(|r| !r.info.invited && r.scope.room_id.as_str() == room.room_id);
                        let act = action(joined, s.joins.get(&room.room_id));
                        if row(ui, s, room, &act, t) {
                            click = Some((room.clone(), joined));
                        }
                    }
                    match m {
                        More::None => {}
                        More::Loading => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                line(ui, more_text(*m), t.secondary);
                            });
                        }
                        _ => more |= ui.button(more_text(*m)).clicked(),
                    }
                });
            }
        }
    });
    if let Some((room, joined)) = click {
        if joined {
            if let Some(scope) = s.sidebar.rooms().iter().find(|r| r.scope.room_id.as_str() == room.room_id).map(|r| r.scope.clone()) {
                s.sidebar.select(&scope);
                open = false;
            }
        } else {
            s.join_public(&room);
        }
    }
    if more {
        s.more_rooms();
    }
    if open {
        s.explore = Some(e);
    } else {
        s.close_explore();
    }
}

/// One directory row, painted like a list row: picture, name, «alias · N membri», topic, and the button. True when the button was used.
fn row(ui: &mut egui::Ui, s: &Session, room: &DirectoryRoom, act: &Action, t: &DesignTokens) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::hover());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 10., rgb(t.input));
    }
    let c = egui::pos2(rect.left() + 28., rect.center().y);
    crate::avatar::paint(ui, c, 40., rgb(matrix_desktop::AVATAR_FALLBACK), &crate::avatar::lookup(&s.avatars, room.avatar_url.as_deref()), &room.name.chars().take(2).collect::<String>(), 15., rgb(t.avatar_text));
    let btn = egui::Rect::from_min_size(egui::pos2(rect.right() - 96., rect.center().y - crate::metrics::button() / 2.), Vec2::new(88., crate::metrics::button()));
    let left = rect.left() + 58.;
    let width = (btn.left() - 8. - left).max(20.);
    let topic = room.topic.as_deref().unwrap_or("");
    let (third, third_color) = match act {
        Action::Retry(why) => (*why, t.error),
        Action::Open => (i18n::t("Already in"), t.accent),
        _ => (topic, t.secondary),
    };
    for (text, size, color, y) in [(room.name.as_str(), 15., t.text, 10.), (&*room.detail(), 12., t.secondary, 30.), (third, 12., third_color, 48.)] {
        if !text.is_empty() {
            let g = one_line(ui, text, size, rgb(color), width);
            ui.painter().galley(egui::pos2(left, rect.top() + y), g, rgb(color));
        }
    }
    let label = match act {
        Action::Open => i18n::t("Open"),
        Action::Join => i18n::t("Join"),
        Action::Joining => matrix_desktop::i18n::t("Joining…"),
        Action::Retry(_) => i18n::t("Retry"),
    };
    ({
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(btn));
        let visuals = &mut ui.style_mut().visuals;
        for widget in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active] {
            widget.bg_stroke = egui::Stroke::NONE;
            widget.expansion = 0.;
            widget.fg_stroke.color = rgb(t.text);
        }
        visuals.widgets.inactive.bg_fill = button_color(t, 22);
        visuals.widgets.inactive.weak_bg_fill = button_color(t, 22);
        visuals.widgets.hovered.bg_fill = button_color(t, 32);
        visuals.widgets.hovered.weak_bg_fill = button_color(t, 32);
        visuals.widgets.active.bg_fill = rgb(t.selected);
        visuals.widgets.active.weak_bg_fill = rgb(t.selected);
        ui.put(btn, egui::Button::new(egui::RichText::new(label).color(rgb(t.text))))
            .on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }) && *act != Action::Joining
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join::Cause;

    #[test]
    fn button_and_secondary_text_contrast_in_all_themes() {
        fn luminance(c: egui::Color32) -> f64 {
            let channel = |n: u8| { let v = f64::from(n) / 255.; if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        }
        fn contrast(a: egui::Color32, b: egui::Color32) -> f64 {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        }
        for t in [matrix_desktop::LIGHT, matrix_desktop::DAY, matrix_desktop::DARK, matrix_desktop::NIGHT] {
            for bg in [button_color(&t, 22), button_color(&t, 32), rgb(t.selected)] {
                assert!(contrast(rgb(t.text), bg) >= 4.5);
            }
            assert!(contrast(rgb(t.secondary), rgb(t.input)) >= 4.5);
            assert!(contrast(rgb(t.input), button_color(&t, 22)) >= 1.25);
        }
    }

    #[test]
    fn request_follows_term_server_and_kind() {
        let mut e = Explore::new();
        assert_eq!(e.request(), Some((String::new(), None, Some(matrix_engine::operations::DirectoryKind::Rooms))));
        e.term = " rust ".into();
        e.pick = Pick::MatrixOrg;
        e.spaces = true;
        assert_eq!(e.request(), Some(("rust".into(), Some("matrix.org".into()), Some(matrix_engine::operations::DirectoryKind::Spaces))));
        e.pick = Pick::Other;
        assert_eq!(e.request(), None); // empty name: nothing asked
        e.other = "https://Chat.Example.org/".into();
        assert_eq!(e.request().and_then(|r| r.1), Some("chat.example.org".into()));
        e.other = "@a:b.c".into();
        assert_eq!(e.request(), None);
        e.pick = Pick::Own;
        e.term = "r".into(); // too short for the server: first page
        assert_eq!(e.request().map(|r| r.0), Some(String::new()));
    }

    #[test]
    fn action_by_membership_and_join_state() {
        assert_eq!(action(true, Some(&JoinState::Failed(Cause::Generic))), Action::Open);
        assert_eq!(action(false, None), Action::Join);
        assert_eq!(action(false, Some(&JoinState::Joining)), Action::Joining);
        assert_eq!(action(false, Some(&JoinState::StillJoining)), Action::Joining);
        assert_eq!(action(false, Some(&JoinState::Failed(Cause::RoomMissing))), Action::Retry("Room not found"));
    }
}
