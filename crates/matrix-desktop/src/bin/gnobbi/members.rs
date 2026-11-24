//! Member administration of a room or a space: «Invita», «Espelli»/«Blocca» with an optional reason, «Bloccati» with «Sblocca».
use crate::list::rgb;
use eframe::egui::{self, Align, Align2, RichText};
use matrix_desktop::{DesignTokens, i18n};
use matrix_domain::RoomScope;
use matrix_engine::operations::{BannedMember, Member, MemberPermissions, Moderation};

pub fn role_label(role: matrix_domain::role::MemberRole) -> &'static str {
    use matrix_domain::role::MemberRole::*;
    match role {
        Admin => i18n::t(i18n::t("Admin")),
        Moderator => i18n::t("Moderator"),
        Member => i18n::t("Member"),
    }
}

/// What the user may do with the members of `scope` and what is going on.
pub struct Admin {
    pub scope: RoomScope,
    /// All false until the engine answers: nothing is offered meanwhile.
    pub perms: MemberPermissions,
    pub banned: Result<Vec<BannedMember>, ()>,
    pub dialog: Option<Dialog>,
    pub inviting: bool,
    /// An action went out and its answer is not in yet.
    pub busy: bool,
    pub failed: bool,
}

/// «Espelli» or «Blocca» waiting for its confirmation.
pub struct Dialog {
    pub user_id: String,
    pub name: String,
    pub ban: bool,
    pub reason: String,
}

pub enum Act {
    Profile(String),
    Ask { user_id: String, name: String, ban: bool },
    Invite,
    Unban(String),
}

impl Admin {
    pub fn new(scope: RoomScope) -> Self {
        Self { scope, perms: Default::default(), banned: Ok(Vec::new()), dialog: None, inviting: false, busy: false, failed: false }
    }

    /// The engine answered an action of ours. True when `user_id` left the member list.
    // The engine's banned list follows the local store (next sync), so the lists are updated here instead of read again.
    pub fn done(&mut self, user_id: &str, action: &Moderation, ok: bool) -> bool {
        // An answer to the profile card's own «Espelli»/«Blocca» moves the user too, but closes nothing here.
        let ours = std::mem::take(&mut self.busy);
        self.failed = ours && !ok;
        if !ok {
            return false;
        }
        let name = if ours { self.dialog.take().map(|d| d.name) } else { None };
        self.inviting &= !ours;
        let reason = match action {
            Moderation::BanWith(reason) => Some(reason.trim().to_owned()).filter(|r| !r.is_empty()),
            _ => None,
        };
        if let (Ok(banned), Moderation::Ban | Moderation::BanWith(_) | Moderation::Unban(_)) = (&mut self.banned, action) {
            banned.retain(|b| b.user_id != user_id);
            if !matches!(action, Moderation::Unban(_)) {
                banned.push(BannedMember { user_id: user_id.to_owned(), name: name.unwrap_or_else(|| user_id.to_owned()), avatar: None, reason });
            }
        }
        matches!(action, Moderation::Kick | Moderation::Ban | Moderation::KickWith(_) | Moderation::BanWith(_))
    }
}

/// Rows of the preview in the info panel; the rest is on the members page.
pub const PREVIEW: usize = 20;

/// Fixed height of a member row, so `show_rows` can place any of them without measuring: the «⋮» button decides on touch.
pub fn row_h() -> f32 {
    crate::metrics::button().max(40.)
}

/// user_id -> most recent message in the loaded timeline (ms). O(events), not O(members).
pub type Activity = std::collections::HashMap<String, u64>;

/// shortcut: only the loaded part of the timeline counts and there is no presence (sliding sync v5 has none); add it here if it arrives.
pub fn activity(events: &[matrix_desktop::DemoEvent], me: &str) -> Activity {
    let mut a = Activity::new();
    for e in events {
        let Some(id) = e.sender.as_ref().map(|s| s.id.as_str()).or(e.outgoing.then_some(me)) else { continue };
        let t = a.entry(id.to_owned()).or_default();
        *t = (*t).max(e.timestamp_ms);
    }
    a
}

#[cfg(test)]
thread_local!(static SORTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });

/// The members of a room or space with everything derived from them (order, duplicate names, search), computed once when the list
/// changes and never per frame. Fields are private so a change cannot leave the derived data stale.
pub struct MemberList {
    pub scope: RoomScope,
    rows: Vec<Member>,
    duplicate: Vec<bool>,
    /// Lowercase "name\nid" per row, for the search.
    keys: Vec<String>,
    /// (events, last timestamp) of the timeline the order was computed with.
    mark: (usize, u64),
    /// Last search asked and its answer (indices into `rows`).
    query: (String, Option<String>),
    hits: Vec<u32>,
}

impl MemberList {
    pub fn new(scope: RoomScope, rows: Vec<Member>, activity: &Activity, mark: (usize, u64)) -> Self {
        let mut l = Self { scope, rows, duplicate: Vec::new(), keys: Vec::new(), mark, query: Default::default(), hits: Vec::new() };
        l.resort(mark, activity);
        l
    }

    pub fn rows(&self) -> &[Member] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn duplicate(&self, i: usize) -> bool {
        self.duplicate[i]
    }

    pub fn mark(&self) -> (usize, u64) {
        self.mark
    }

    pub fn resort(&mut self, mark: (usize, u64), activity: &Activity) {
        #[cfg(test)]
        SORTS.with(|c| c.set(c.get() + 1));
        self.rows.sort_by_cached_key(|m| matrix_domain::role::member_sort_key(m.role, activity.get(&m.user_id).copied(), &m.name));
        self.mark = mark;
        self.derive();
    }

    pub fn retain(&mut self, keep: impl FnMut(&Member) -> bool) {
        self.rows.retain(keep);
        self.derive();
    }

    pub fn set_role(&mut self, user_id: &str, role: matrix_domain::role::MemberRole, activity: &Activity) {
        self.rows.iter_mut().filter(|m| m.user_id == user_id).for_each(|m| m.role = role);
        self.resort(self.mark, activity);
    }

    fn derive(&mut self) {
        self.duplicate = crate::rows::duplicate_names(self.rows.iter().map(|m| m.name.as_str()));
        self.keys = self.rows.iter().map(|m| format!("{}\n{}", m.name, m.user_id).to_lowercase()).collect();
        self.query = Default::default();
        self.hits = (0..self.rows.len() as u32).collect();
    }

    /// Rows whose name or id contain `query` (case-insensitive), `without` excluded. Recomputed only when the question changed.
    pub fn refresh_filter(&mut self, query: &str, without: Option<&str>) {
        let q = query.trim().to_lowercase();
        if self.query.0 == q && self.query.1.as_deref() == without {
            return;
        }
        self.hits = (0..self.rows.len() as u32).filter(|&i| self.keys[i as usize].contains(&q) && without != Some(self.rows[i as usize].user_id.as_str())).collect();
        self.query = (q, without.map(str::to_owned));
    }

    pub fn hits(&self) -> &[u32] {
        &self.hits
    }
}

fn invite(ui: &mut egui::Ui, perms: MemberPermissions) -> Option<Act> {
    let mut act = None;
    if perms.can_invite {
        ui.add_space(8.);
        ui.horizontal(|ui| {
            ui.add_space(14.);
            let invite = ui.button(crate::icons::with_label(crate::icons::Icon::UserPlus, i18n::t("Invite"))).on_hover_text(i18n::t("Invite"));
            invite.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, i18n::t("Invite")));
            if invite.clicked() {
                act = Some(Act::Invite);
            }
        });
        ui.add_space(4.);
    }
    act
}

/// One member row of fixed height, with its «⋮» menu when the user may moderate.
fn row(ui: &mut egui::Ui, m: &Member, duplicate: bool, perms: MemberPermissions, me: &str, hook: &mut bool, avatars: &crate::avatar::Avatars, t: &DesignTokens) -> Option<Act> {
    let mut act = None;
    let menu = (perms.can_kick || perms.can_ban) && m.user_id != me;
    ui.push_id(&m.user_id, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.;
            ui.with_layout(crate::dir::row_rev(Align::Center), |ui| {
                if menu {
                    ui.add_space(8.);
                    let resp = crate::space::dots(ui, t, &i18n::t("Actions for {name}").replace("{name}", &m.name));
                    let mut popup = egui::Popup::menu(&resp);
                    if std::mem::take(hook) {
                        popup = popup.open(true);
                    }
                    popup.show(|ui| crate::dir::popup(ui, |ui| {
                        let mut ask = |ui: &mut egui::Ui, label: &str, ban| {
                            if ui.button(label).clicked() {
                                act = Some(Act::Ask { user_id: m.user_id.clone(), name: m.name.clone(), ban });
                                ui.close();
                            }
                        };
                        if perms.can_kick {
                            ask(ui, i18n::t("Kick"), false);
                        }
                        if perms.can_ban {
                            ask(ui, i18n::t("Ban"), true);
                        }
                    }));
                } else if perms.can_kick || perms.can_ban {
                    // The own row has no «⋮»: its place stays empty so the badges line up.
                    ui.add_space(40.);
                }
                if crate::info::member_row(ui, avatars, m, duplicate, t).clicked() {
                    act = Some(Act::Profile(m.user_id.clone()));
                }
            });
        });
    });
    act
}

/// Test hook: GNOBBI_EGUI_MEMBER=menu shows the first «⋮» menu without a click.
fn menu_hook() -> bool {
    matrix_desktop::hooks::var("GNOBBI_EGUI_MEMBER").as_deref() == Ok("menu")
}

/// Tells `drive_avatars` which pictures are on screen: only those are requested.
fn publish_avatars(ui: &egui::Ui, rows: impl Iterator<Item = Option<String>>) {
    let visible: Vec<String> = rows.flatten().collect();
    ui.data_mut(|d| d.insert_temp(egui::Id::new(crate::avatar::MEMBER_AVATARS), visible));
}

/// The first `PREVIEW` members (admins and moderators come first) and «Members · N» when there are more, then «Bloccati».
/// `all` is set when «Members · N» is clicked.
pub fn preview(ui: &mut egui::Ui, list: &MemberList, admin: Option<&Admin>, me: &str, all: &mut bool, avatars: &crate::avatar::Avatars, t: &DesignTokens) -> Option<Act> {
    let perms = admin.map(|a| a.perms).unwrap_or_default();
    let mut act = invite(ui, perms);
    let mut hook = menu_hook();
    let shown = list.len().min(PREVIEW);
    for (m, i) in list.rows().iter().zip(0..shown) {
        act = row(ui, m, list.duplicate(i), perms, me, &mut hook, avatars, t).or(act);
    }
    publish_avatars(ui, list.rows()[..shown].iter().map(|m| m.avatar.clone()));
    if list.len() > shown {
        let n = list.len() as u64;
        let line = ui.horizontal(|ui| {
            ui.set_min_height(row_h());
            ui.add_space(14.);
            ui.label(RichText::new(i18n::t("Members")).strong());
            ui.with_layout(crate::dir::row_rev(Align::Center), |ui| {
                ui.add_space(14.);
                ui.label(RichText::new(n.to_string()).color(rgb(t.secondary)));
            });
        }).response;
        let resp = crate::metrics::clickable(ui.interact(line.rect, ui.id().with("members-all"), egui::Sense::click()));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, i18n::plural(n, "{n} member", "{n} members")));
        *all |= resp.clicked();
    }
    banned(ui, admin, avatars, t).or(act)
}

pub struct PageOut {
    pub act: Option<Act>,
    pub back: bool,
}

/// All the members, searchable, drawn with `show_rows`: only the rows on screen cost anything, whatever the count.
/// With `promote` each row offers «Rendi amministratore» instead of the menu (the user's own row is left out).
pub fn page(ui: &mut egui::Ui, list: &mut MemberList, query: &mut String, promote: Option<&mut Option<String>>, admin: Option<&Admin>, me: &str, avatars: &crate::avatar::Avatars, t: &DesignTokens) -> PageOut {
    let perms = admin.map(|a| a.perms).unwrap_or_default();
    let mut out = PageOut { act: None, back: false };
    ui.horizontal(|ui| {
        out.back = crate::icons::button(ui, crate::icons::Icon::CaretLeft, false, i18n::t("Back")).clicked();
        ui.label(RichText::new(i18n::t("Members")).strong());
        ui.label(RichText::new(list.len().to_string()).color(rgb(t.secondary)));
    });
    ui.horizontal(|ui| {
        ui.add_space(12.);
        let w = ui.available_width() - 12.;
        crate::metrics::search(ui, [w, crate::metrics::button()], query, i18n::t("Search"));
    });
    ui.add_space(4.);
    if promote.is_none() {
        out.act = invite(ui, perms);
    }
    list.refresh_filter(query, promote.is_some().then_some(me));
    let (rows, hits) = (list.rows(), list.hits());
    let mut promote = promote;
    let mut hook = menu_hook();
    let mut visible = Vec::new();
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_h(), hits.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.;
        for &i in &hits[range] {
            let m = &rows[i as usize];
            visible.push(m.avatar.clone());
            if let Some(promote) = promote.as_deref_mut() {
                ui.horizontal(|ui| {
                    ui.set_min_height(row_h());
                    ui.add(egui::Label::new(crate::list::clean_single_line(&m.name)).truncate());
                    if ui.button(i18n::t("Make admin")).clicked() {
                        *promote = Some(m.user_id.clone());
                    }
                });
            } else {
                out.act = row(ui, m, list.duplicate(i as usize), perms, me, &mut hook, avatars, t).or(out.act.take());
            }
        }
    });
    publish_avatars(ui, visible.into_iter());
    out
}

/// «Bloccati» and the failure line.
fn banned(ui: &mut egui::Ui, admin: Option<&Admin>, avatars: &crate::avatar::Avatars, t: &DesignTokens) -> Option<Act> {
    let mut act = None;
    let perms = admin.map(|a| a.perms).unwrap_or_default();
    let busy = admin.is_some_and(|a| a.busy);
    let Some(admin) = admin.filter(|a| a.perms.can_ban) else { return act };
    if admin.banned.as_ref().is_ok_and(|rows| rows.is_empty()) {
        return act;
    }
    ui.add_space(8.);
    ui.separator();
    ui.horizontal(|ui| {
        ui.add_space(14.);
        ui.label(RichText::new(i18n::t("Banned")).strong());
    });
    let Ok(rows) = &admin.banned else {
        ui.horizontal(|ui| {
            ui.add_space(14.);
            ui.colored_label(rgb(t.error), i18n::t("Banned list unavailable"));
        });
        return act;
    };
    let duplicates = crate::rows::duplicate_names(rows.iter().map(|b| b.name.as_str()));
    for (b, duplicate) in rows.iter().zip(duplicates) {
        if duplicate {
            let name = crate::list::clean_single_line(&b.name);
            let id = crate::list::clean_single_line(&b.user_id);
            let reason = b.reason.as_deref().map(crate::list::clean_single_line);
            let mut lines = vec![crate::rows::Line::new(&name, 14., rgb(t.text)), crate::rows::Line::new(&id, 12., rgb(t.secondary))];
            if let Some(reason) = &reason { lines.push(crate::rows::Line::new(reason, 12., rgb(t.secondary))); }
            crate::rows::lines_row(ui, |ui| {
                ui.add_space(14.);
                let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(28.), egui::Sense::hover());
                crate::avatar::paint(ui, r.center(), 28., rgb(matrix_desktop::AVATAR_FALLBACK), &crate::avatar::lookup(avatars, b.avatar.as_deref()), &matrix_desktop::initials(&b.name), 12., rgb(t.on_accent));
            }, &lines, |ui| {
                if perms.can_kick && ui.add_enabled(!busy, egui::Button::new(i18n::t("Unban"))).clicked() { act = Some(Act::Unban(b.user_id.clone())); }
            });
            continue;
        }
        ui.horizontal(|ui| {
            ui.set_min_height(40.);
            ui.add_space(14.);
            ui.with_layout(crate::dir::row_rev(Align::Center), |ui| {
                ui.add_space(8.);
                // Unban needs the kick level on top of the ban level.
                if perms.can_kick {
                    if ui.add_enabled(!busy, egui::Button::new(i18n::t("Unban"))).clicked() {
                        act = Some(Act::Unban(b.user_id.clone()));
                    }
                }
                let name = egui::Label::new(RichText::new(crate::list::clean_single_line(&b.name)).color(rgb(t.text))).truncate();
                match &b.reason {
                    Some(reason) => drop(ui.with_layout(crate::dir::col(), |ui| {
                        ui.add_space(3.);
                        ui.add(name);
                        ui.add(egui::Label::new(RichText::new(crate::list::clean_single_line(reason)).size(12.).color(rgb(t.secondary))).truncate());
                    })),
                    None => drop(ui.with_layout(crate::dir::row(Align::Center), |ui| ui.add(name))),
                }
            });
        });
    }
    if admin.failed && admin.dialog.is_none() && !admin.inviting {
        ui.horizontal(|ui| {
            ui.add_space(14.);
            ui.colored_label(rgb(t.error), i18n::t("Failed"));
        });
    }
    act
}

/// The confirmation with its reason and the «Invita» search, over whichever member list is showing.
pub fn windows(ctx: &egui::Context, s: &mut crate::session::Session, t: &DesignTokens) {
    // The list on screen decides whose rights are read: the space window covers the room panel.
    let wanted = match &s.space_view {
        Some(v) if v.members_page => Some(v.scope.clone()),
        _ => s.sidebar.selected().filter(|_| s.info_open).map(|r| r.scope.clone()),
    };
    s.admin_for(wanted);
    let Some(mut admin) = s.admin.take() else { return };
    let mut send = None;
    if let Some(d) = &mut admin.dialog {
        let (verb, mut cancel) = (if d.ban { i18n::t("Ban") } else { i18n::t("Kick") }, false);
        egui::Window::new(verb).id(egui::Id::new("member-dialog")).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
            ui.set_width(crate::dir::fit(ui, 340.));
            ui.add(egui::Label::new(RichText::new(crate::list::clean_single_line(&d.name)).strong()).truncate());
            ui.add_space(4.);
            ui.add_enabled_ui(!admin.busy, |ui| ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut d.reason).hint_text(i18n::t("Reason"))));
            if admin.failed {
                ui.colored_label(rgb(t.error), i18n::t("Failed"));
            }
            ui.add_space(8.);
            ui.horizontal(|ui| {
                if admin.busy {
                    ui.spinner();
                    return;
                }
                if ui.button(RichText::new(verb).color(rgb(t.error))).clicked() {
                    let reason = d.reason.trim().to_owned();
                    send = Some((d.user_id.clone(), if d.ban { Moderation::BanWith(reason) } else { Moderation::KickWith(reason) }));
                }
                cancel = ui.button(i18n::t("Cancel")).clicked();
            });
        }));
        if cancel {
            (admin.dialog, admin.failed) = (None, false);
        }
    } else if admin.inviting {
        let mut open = true;
        egui::Window::new(i18n::t("Invite")).id(egui::Id::new("member-invite")).open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
            ui.set_width(crate::dir::fit(ui, 340.));
            ui.add_enabled_ui(!admin.busy, |ui| {
                if let Some(user) = crate::newchat::people(ui, s, t) {
                    send = Some((user, Moderation::Invite));
                }
            });
            if admin.busy {
                ui.spinner();
            } else if admin.failed {
                ui.colored_label(rgb(t.error), i18n::t("Failed"));
            }
        }));
        if !open {
            (admin.inviting, admin.failed) = (false, false);
        }
    }
    if let Some((user_id, action)) = send {
        (admin.busy, admin.failed) = (true, false);
        s.moderate_in(admin.scope.clone(), user_id, action);
    }
    s.admin = Some(admin);
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId};

    #[test]
    fn answers_move_the_user_between_the_lists() {
        let mut a = Admin::new(RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:x") });
        // A failure that is not ours (sent from the profile card) shows nothing here.
        assert!(!a.done("@u:x", &Moderation::Ban, false));
        assert!(!a.failed && a.banned.as_ref().unwrap().is_empty());
        let ask = |a: &mut Admin| {
            a.busy = true;
            a.dialog = Some(Dialog { user_id: "@u:x".into(), name: "Ugo".into(), ban: true, reason: String::new() });
        };
        ask(&mut a);
        assert!(!a.done("@u:x", &Moderation::BanWith(" spam ".into()), false));
        assert!(a.failed && a.dialog.is_some() && !a.busy);
        ask(&mut a);
        assert!(a.done("@u:x", &Moderation::BanWith(" spam ".into()), true));
        let b = &a.banned.as_ref().unwrap()[0];
        assert_eq!((b.name.as_str(), b.reason.as_deref()), ("Ugo", Some("spam")));
        assert!(a.dialog.is_none() && !a.failed);
        a.busy = true;
        assert!(!a.done("@u:x", &Moderation::Unban(None), true));
        assert!(a.banned.unwrap().is_empty());
    }

    fn list_of(n: usize, avatar: bool) -> MemberList {
        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:x") };
        MemberList::new(scope, crate::members_bench::synthetic(n, avatar), &Activity::new(), (0, 0))
    }

    fn frame(ctx: &egui::Context, list: &mut MemberList, query: &mut String) {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 800.))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            page(ui, list, query, None, None, "@me:x", &Default::default(), &matrix_desktop::LIGHT);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn sorting_happens_once_and_never_per_frame() {
        let sorts = || SORTS.with(|c| c.get());
        let before = sorts();
        let mut list = list_of(8000, true);
        assert_eq!(sorts(), before + 1);
        let (ctx, mut q) = (egui::Context::default(), String::new());
        for _ in 0..10 {
            frame(&ctx, &mut list, &mut q);
        }
        assert_eq!(sorts(), before + 1);
        // Activity puts the member who wrote last first inside the group; the same mark is a no-op for the caller, a new one sorts again.
        let activity = Activity::from([("@user9000:x".to_owned(), 5), ("@user77:example.org".to_owned(), 9)]);
        list.resort((1, 9), &activity);
        assert_eq!(list.mark(), (1, 9));
        assert_eq!(list.rows()[10].user_id, "@user77:example.org");
        assert_eq!(sorts(), before + 2);
    }

    #[test]
    fn search_finds_name_and_id_and_leaves_the_user_out() {
        let mut list = list_of(200, false);
        list.refresh_filter("NUMERO 15", None);
        assert!(list.hits().iter().all(|&i| list.rows()[i as usize].name.to_lowercase().contains("numero 15")) && list.hits().len() == 10);
        list.refresh_filter("user7:example", None);
        assert_eq!(list.hits().iter().map(|&i| list.rows()[i as usize].user_id.as_str()).collect::<Vec<_>>(), ["@user7:example.org"]);
        list.refresh_filter("", Some("@user7:example.org"));
        assert_eq!(list.hits().len(), 199);
    }

    #[test]
    fn retain_rebuilds_the_derived_data() {
        let mut list = list_of(100, false);
        assert!(list.duplicate(list.rows().iter().position(|m| m.name == "Marco").unwrap()));
        list.refresh_filter("marco", None);
        assert_eq!(list.hits().len(), 2);
        list.retain(|m| m.user_id != "@user0:example.org");
        assert_eq!((list.len(), list.hits().len()), (99, 99));
        list.refresh_filter("marco", None);
        assert_eq!(list.hits().len(), 1);
        assert!(!list.duplicate(list.rows().iter().position(|m| m.name == "Marco").unwrap()));
    }

    #[test]
    fn only_the_visible_rows_ask_for_their_avatar() {
        let mut list = list_of(8000, true);
        let (ctx, mut q) = (egui::Context::default(), String::new());
        frame(&ctx, &mut list, &mut q);
        let visible = ctx.data(|d| d.get_temp::<Vec<String>>(egui::Id::new(crate::avatar::MEMBER_AVATARS))).unwrap();
        assert!(!visible.is_empty() && visible.len() <= (800. / row_h()).ceil() as usize + 1, "{}", visible.len());
        assert!(visible.contains(list.rows()[0].avatar.as_ref().unwrap()) && !visible.contains(list.rows()[7999].avatar.as_ref().unwrap()));
    }
}
