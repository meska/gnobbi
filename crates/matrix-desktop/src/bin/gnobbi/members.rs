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

/// «Invita», the member rows with their «⋮» menu, then «Bloccati». Only what the user's power level allows is drawn.
pub fn list(ui: &mut egui::Ui, members: &[Member], admin: Option<&Admin>, me: &str, avatars: &crate::avatar::Avatars, t: &DesignTokens) -> Option<Act> {
    let mut act = None;
    let perms = admin.map(|a| a.perms).unwrap_or_default();
    let busy = admin.is_some_and(|a| a.busy);
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
    // Test hook: GNOBBI_EGUI_MEMBER=menu shows the first «⋮» menu without a click.
    let mut hook = matrix_desktop::hooks::var("GNOBBI_EGUI_MEMBER").as_deref() == Ok("menu");
    let duplicates = crate::rows::duplicate_names(members.iter().map(|m| m.name.as_str()));
    for (m, duplicate) in members.iter().zip(duplicates) {
        let menu = (perms.can_kick || perms.can_ban) && m.user_id != me;
        ui.push_id(&m.user_id, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.;
                ui.with_layout(crate::dir::row_rev(Align::Center), |ui| {
                    if menu {
                        ui.add_space(8.);
                        let resp = crate::space::dots(ui, t, &i18n::t("Actions for {name}").replace("{name}", &m.name));
                        let mut popup = egui::Popup::menu(&resp);
                        if std::mem::take(&mut hook) {
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
    }
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
            ui.set_width(340.);
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
            ui.set_width(340.);
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
}
