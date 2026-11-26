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

/// Order mark of a list: (events, last timestamp, hash of the read receipts) of the timeline it was computed with.
pub type Mark = (usize, u64, u64);

/// shortcut: only the loaded part of the timeline counts (a receipt on an event not loaded is ignored) and there is no presence (sliding sync v5 has none); add it here if it arrives.
pub fn activity(events: &[matrix_desktop::DemoEvent], receipts: &[matrix_engine::receipts::PublicReadReceipt], me: &str) -> Activity {
    let server = |e: &matrix_desktop::DemoEvent| match &e.id.id { matrix_domain::TimelineId::Server(id) => Some(id.as_str().to_owned()), _ => None };
    activity_of(events.iter().map(|e| (e.sender.as_ref().map(|s| s.id.as_str()).or(e.outgoing.then_some(me)), e.timestamp_ms, server(e))), receipts)
}

/// Last message per author, and the moment of the event each user's read receipt sits on, whichever is later.
fn activity_of<'a>(events: impl Iterator<Item = (Option<&'a str>, u64, Option<String>)>, receipts: &[matrix_engine::receipts::PublicReadReceipt]) -> Activity {
    let readers: std::collections::HashMap<&str, &Vec<matrix_engine::history::HistorySender>> = receipts.iter().map(|r| (r.event_id.as_str(), &r.users)).collect();
    let mut a = Activity::new();
    let mut seen = |id: &str, ts: u64| {
        let t = a.entry(id.to_owned()).or_default();
        *t = (*t).max(ts);
    };
    for (author, ts, event) in events {
        author.into_iter().for_each(|id| seen(id, ts));
        event.and_then(|id| readers.get(id.as_str()).copied()).into_iter().flatten().for_each(|u| seen(&u.id, ts));
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
    /// (events, last timestamp, receipts) of the timeline the order was computed with.
    mark: Mark,
    /// Last search asked and its answer (indices into `rows`).
    query: (String, Option<String>),
    hits: Vec<u32>,
}

impl MemberList {
    pub fn new(scope: RoomScope, rows: Vec<Member>, activity: &Activity, mark: Mark) -> Self {
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

    pub fn mark(&self) -> Mark {
        self.mark
    }

    pub fn resort(&mut self, mark: Mark, activity: &Activity) {
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
    // Added to, not replaced: a space window over a room with the info open draws two lists in one frame.
    ui.data_mut(|d| {
        let wanted = d.get_temp_mut_or_default::<Vec<String>>(egui::Id::new(crate::avatar::MEMBER_AVATARS));
        for v in visible {
            if !wanted.contains(&v) {
                wanted.push(v);
            }
        }
    });
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
    if hits.is_empty() && list.len() != 0 {
        ui.label(RichText::new(i18n::t("No results")).color(rgb(t.secondary)));
    }
    // Rows are row_h() apart: `show_rows` takes its step from the spacing of the Ui it is called on, not the one inside.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_h(), hits.len(), |ui, range| {
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
    });
    publish_avatars(ui, visible.into_iter());
    out
}

/// True when `banned` draws something.
pub fn has_banned(admin: Option<&Admin>) -> bool {
    admin.is_some_and(|a| a.perms.can_ban && !a.banned.as_ref().is_ok_and(|r| r.is_empty()))
}

/// «Bloccati» and the failure line.
pub fn banned(ui: &mut egui::Ui, admin: Option<&Admin>, avatars: &crate::avatar::Avatars, t: &DesignTokens) -> Option<Act> {
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
        MemberList::new(scope, crate::members_bench::synthetic(n, avatar), &Activity::new(), (0, 0, 0))
    }

    fn frame(ctx: &egui::Context, list: &mut MemberList, query: &mut String) {
        // The session drains the list of wanted pictures every frame.
        ctx.data_mut(|d| d.remove_temp::<Vec<String>>(egui::Id::new(crate::avatar::MEMBER_AVATARS)));
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
        list.resort((1, 9, 0), &activity);
        assert_eq!(list.mark(), (1, 9, 0));
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

    fn visible_avatars(ctx: &egui::Context) -> Vec<String> {
        ctx.data(|d| d.get_temp::<Vec<String>>(egui::Id::new(crate::avatar::MEMBER_AVATARS))).unwrap()
    }

    /// One frame of the page in a 300x`h` window with `events` (wheel, pointer) and the given avatar map.
    fn frame_with(ctx: &egui::Context, list: &mut MemberList, query: &mut String, h: f32, events: Vec<egui::Event>, avatars: &crate::avatar::Avatars) -> Vec<egui::epaint::ClippedShape> {
        // The session drains the list of wanted pictures every frame.
        ctx.data_mut(|d| d.remove_temp::<Vec<String>>(egui::Id::new(crate::avatar::MEMBER_AVATARS)));
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., h))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            page(ui, list, query, None, None, "@me:x", avatars, &matrix_desktop::LIGHT);
        });
        out.textures_delta.clear();
        out.shapes
    }

    /// Scroll the list by `dy` points (positive = down) and let egui's scroll smoothing settle.
    fn scroll(ctx: &egui::Context, list: &mut MemberList, query: &mut String, h: f32, dy: f32) {
        let wheel = vec![egui::Event::PointerMoved(egui::pos2(150., h / 2.)), egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0., -dy), modifiers: Default::default(), phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Unknown }];
        frame_with(ctx, list, query, h, wheel, &Default::default());
        for _ in 0..30 {
            frame_with(ctx, list, query, h, vec![], &Default::default());
        }
    }

    fn median_frame(n: usize) -> std::time::Duration {
        let mut list = list_of(n, true);
        let (ctx, mut q) = (egui::Context::default(), String::new());
        let mut times: Vec<_> = (0..14)
            .map(|_| {
                let t = std::time::Instant::now();
                frame(&ctx, &mut list, &mut q);
                t.elapsed()
            })
            .skip(4)
            .collect();
        times.sort();
        times[times.len() / 2]
    }

    fn shapes_drawn(n: usize) -> usize {
        let mut list = list_of(n, true);
        let (ctx, mut q) = (egui::Context::default(), String::new());
        frame_with(&ctx, &mut list, &mut q, 800., vec![], &Default::default());
        frame_with(&ctx, &mut list, &mut q, 800., vec![], &Default::default()).len()
    }

    /// Absolute cap 50 ms on the median of 10 warm frames: 8000 rows drawn the old way cost 27-39 ms already in RELEASE, the virtualised page ~1 ms in debug.
    /// That the cost does not grow with N is checked without a clock: the number of shapes of a frame is about the same for 100 and for 8000 members.
    #[test]
    fn frame_with_8000_members_is_fast_and_does_not_grow_with_n() {
        let big = median_frame(8000);
        eprintln!("median frame, 8000 members: {big:?}");
        assert!(big < std::time::Duration::from_millis(50), "8000 members: {big:?}");
        let small = shapes_drawn(100);
        assert!(small > 0);
        // Not equal to the unit (duplicate names take two lines), but never 80x more.
        assert!(shapes_drawn(8000) < small * 2);
    }

    #[test]
    fn search_is_computed_once_per_question_not_per_frame() {
        let mut list = list_of(8000, true);
        let (ctx, mut q) = (egui::Context::default(), "numero 79".to_owned());
        frame(&ctx, &mut list, &mut q);
        let (ptr, n) = (list.hits().as_ptr(), list.hits().len());
        assert!(n > 0 && n < 8000);
        for _ in 0..10 {
            frame(&ctx, &mut list, &mut q);
        }
        // Same buffer and same content: not even re-allocated, let alone recomputed.
        assert_eq!((list.hits().as_ptr(), list.hits().len()), (ptr, n));
        q.push('1');
        frame(&ctx, &mut list, &mut q);
        assert!(list.hits().len() < n);
        // A change of members resets the answer to "everything"; the next frame filters again.
        list.retain(|_| true);
        assert_eq!(list.hits().len(), 8000);
        frame(&ctx, &mut list, &mut q);
        assert!(list.hits().len() < 8000);
    }

    #[test]
    fn scrolling_asks_only_for_the_rows_on_screen() {
        let mut list = list_of(8000, true);
        let (ctx, mut q) = (egui::Context::default(), String::new());
        let max = (800. / row_h()).ceil() as usize + 2;
        frame(&ctx, &mut list, &mut q);
        assert!(visible_avatars(&ctx).len() <= max);
        scroll(&ctx, &mut list, &mut q, 800., 100. * row_h());
        let v = visible_avatars(&ctx);
        assert!(!v.is_empty() && v.len() <= max, "{}", v.len());
        // Scrolled away from the top: the first row is no longer asked for, the first on screen is near row 100.
        assert!(!v.contains(list.rows()[0].avatar.as_ref().unwrap()));
        let first = list.rows().iter().position(|m| m.avatar.as_ref() == Some(&v[0])).unwrap();
        assert!((99..=101).contains(&first), "first visible row {first}");
    }

    #[test]
    fn no_members_one_member_and_no_results() {
        let (ctx, mut q) = (egui::Context::default(), String::new());
        let mut empty = list_of(0, true);
        frame(&ctx, &mut empty, &mut q);
        assert!(visible_avatars(&ctx).is_empty() && empty.hits().is_empty());
        let mut one = list_of(1, true);
        frame(&ctx, &mut one, &mut q);
        assert_eq!(visible_avatars(&ctx), ["mxc://example.org/0"]);
        let mut list = list_of(8000, true);
        q.push_str("nessuno-si-chiama-cosi");
        frame(&ctx, &mut list, &mut q);
        assert!(list.hits().is_empty() && visible_avatars(&ctx).is_empty());
        // Back from zero results: the list is whole again.
        q.clear();
        frame(&ctx, &mut list, &mut q);
        assert_eq!(list.hits().len(), 8000);
        assert!(!visible_avatars(&ctx).is_empty());
    }

    #[test]
    fn members_without_avatar_request_nothing() {
        let mut list = list_of(8000, false);
        let (ctx, mut q) = (egui::Context::default(), String::new());
        frame(&ctx, &mut list, &mut q);
        assert!(visible_avatars(&ctx).is_empty());
    }

    #[test]
    fn avatars_still_loading_are_requested_again_only_by_the_driver_and_do_not_break_the_frame() {
        let mut list = list_of(8000, true);
        let (ctx, mut q) = (egui::Context::default(), String::new());
        let loading: crate::avatar::Avatars = list.rows().iter().take(50).filter_map(|m| m.avatar.clone()).map(|a| (a, crate::avatar::Avatar::Loading)).collect();
        frame_with(&ctx, &mut list, &mut q, 800., vec![], &loading);
        // Loading rows stay in the wanted list (the driver skips what is already in the map); rows not on screen never enter it.
        let v = visible_avatars(&ctx);
        assert!(v.contains(list.rows()[0].avatar.as_ref().unwrap()) && v.len() < 50);
        assert!(matches!(crate::avatar::lookup(&loading, list.rows()[0].avatar.as_deref()), crate::avatar::Avatar::Loading));
        assert!(matches!(crate::avatar::lookup(&loading, list.rows()[7999].avatar.as_deref()), crate::avatar::Avatar::Loading));
    }

    #[test]
    fn members_change_while_scrolled_down() {
        let mut list = list_of(8000, true);
        let (ctx, mut q) = (egui::Context::default(), String::new());
        scroll(&ctx, &mut list, &mut q, 800., 5000. * row_h());
        assert!(!visible_avatars(&ctx).contains(list.rows()[0].avatar.as_ref().unwrap()));
        // The room shrinks to 5 members under the scroll offset: the frame must not panic and must show what exists.
        list.retain(|m| m.user_id.trim_start_matches("@user").split(':').next().unwrap().parse::<usize>().unwrap() < 5);
        for _ in 0..3 {
            frame(&ctx, &mut list, &mut q);
        }
        let v = visible_avatars(&ctx);
        assert_eq!(v.len(), 5, "{v:?}");
        // And it grows again with a new sort.
        list.resort((9, 9, 0), &Activity::new());
        frame(&ctx, &mut list, &mut q);
        assert_eq!(visible_avatars(&ctx).len(), 5);
    }

    /// y of the first text shape that contains `needle`.
    fn text_y(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<f32> {
        text_pos(shapes, needle).map(|p| p.y)
    }

    fn text_pos(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<egui::Pos2> {
        fn walk(s: &egui::Shape, needle: &str) -> Option<egui::Pos2> {
            match s {
                egui::Shape::Text(t) if t.galley.text().contains(needle) => Some(t.pos),
                egui::Shape::Vec(v) => v.iter().find_map(|s| walk(s, needle)),
                _ => None,
            }
        }
        shapes.iter().find_map(|c| walk(&c.shape, needle))
    }

    #[test]
    fn one_point_of_wheel_moves_a_row_exactly_one_point() {
        let mut list = list_of(8000, false);
        let (ctx, mut q) = (egui::Context::default(), String::new());
        let name = list.rows()[5].name.clone();
        let wheel = |h: f32| vec![egui::Event::PointerMoved(egui::pos2(150., h / 2.)), egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0., -1.), modifiers: Default::default(), phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Unknown }];
        let mut y = text_y(&frame_with(&ctx, &mut list, &mut q, 300., vec![], &Default::default()), &name).unwrap();
        // Over 90 steps `min_row` advances twice: the old pitch (43 for rows of 40) made the list jump back by 3 pt each time.
        for step in 0..90 {
            let mut shapes = frame_with(&ctx, &mut list, &mut q, 300., wheel(300.), &Default::default());
            for _ in 0..30 {
                shapes = frame_with(&ctx, &mut list, &mut q, 300., vec![], &Default::default());
            }
            let now = text_y(&shapes, &name).unwrap();
            assert!((now - y + 1.).abs() < 0.01, "step {step}: row moved {} pt", now - y);
            y = now;
        }
    }

    #[test]
    fn the_banned_section_shows_and_unban_is_pressed() {
        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:x") };
        let mut admin = Admin::new(scope);
        admin.perms = MemberPermissions { can_kick: true, can_ban: true, ..Default::default() };
        admin.banned = Ok(vec![BannedMember { user_id: "@bad:x".into(), name: "Bad".into(), avatar: None, reason: None }]);
        assert!(has_banned(Some(&admin)));
        let (ctx, mut act) = (egui::Context::default(), None);
        let mut run = |ctx: &egui::Context, events: Vec<egui::Event>, act: &mut Option<Act>| {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 400.))), events, ..Default::default() };
            {
                let mut out = ctx.run_ui(input, |ui| *act = banned(ui, Some(&admin), &Default::default(), &matrix_desktop::LIGHT).or(act.take()));
                out.textures_delta.clear();
                out.shapes
            }
        };
        let shapes = run(&ctx, vec![], &mut act);
        assert!(text_y(&shapes, "Bad").is_some() && text_y(&shapes, i18n::t("Unban")).is_some());
        // Press «Unban»: move, press, release over its label.
        let pos = text_pos(&shapes, i18n::t("Unban")).unwrap() + egui::vec2(4., 4.);
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(&ctx, vec![egui::Event::PointerMoved(pos), button(true)], &mut act);
        run(&ctx, vec![button(false)], &mut act);
        assert!(matches!(&act, Some(Act::Unban(u)) if u == "@bad:x"), "no Unban");
        admin.banned = Ok(vec![]);
        assert!(!has_banned(Some(&admin)));
    }

    fn receipt(event: &str, users: &[&str]) -> matrix_engine::receipts::PublicReadReceipt {
        let users = users.iter().map(|u| matrix_engine::history::HistorySender { id: (*u).into(), name: None, avatar: None }).collect::<Vec<_>>();
        matrix_engine::receipts::PublicReadReceipt { event_id: matrix_domain::EventId::new(event), readers: users.len() as u64, users }
    }

    #[test]
    fn activity_is_the_latest_message_or_the_latest_read_receipt() {
        let events = [(Some("@w:x"), 10, Some("$a:x")), (Some("@w:x"), 30, Some("$b:x")), (Some("@w:x"), 20, None), (Some("@me:x"), 40, None), (None, 25, Some("$c:x"))];
        let a = activity_of(events.into_iter().map(|(a, t, e)| (a, t, e.map(str::to_owned))), &[receipt("$a:x", &["@r:x"]), receipt("$c:x", &["@r:x", "@w:x"])]);
        // The sender's latest wins over the earlier ones; the own message has no sender but counts for `me`; a reader counts at the event read.
        assert_eq!((a["@w:x"], a["@me:x"], a["@r:x"]), (30, 40, 25));
        assert!(!a.contains_key("@nobody:x"));
        // Sorted: who wrote after sits above who only read, who only read above who did nothing.
        let key = |u: &str, name: &str| matrix_domain::role::member_sort_key(matrix_domain::role::MemberRole::Member, a.get(u).copied(), name);
        assert!(key("@w:x", "Z") < key("@r:x", "Y") && key("@r:x", "Y") < key("@none:x", "A"));
    }
}
