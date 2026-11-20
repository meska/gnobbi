//! The selected space: the «⋮» menu of the list header and the window behind it (facts and «Stanze», or the members).
use matrix_desktop::i18n;
use crate::list::rgb;
use eframe::egui::{self, Align2, RichText, Sense, Vec2};
use matrix_desktop::{DesignTokens, SidebarRoom};
use matrix_domain::RoomScope;
use matrix_engine::operations::{Member, SpaceChildren};

pub struct SpaceView {
    pub scope: RoomScope,
    pub members_page: bool,
    /// `None` until the engine answers; an error shows one line.
    pub children: Option<Result<SpaceChildren, ()>>,
    pub members: Option<Vec<Member>>,
    /// «Aggiungi stanza esistente» is open, with the room picked so far.
    pub adding: Option<Option<RoomScope>>,
    /// The members page offers «Rendi amministratore» (opened by «Passa il ruolo di amministratore…»).
    pub promote: bool,
}

/// The vertical three-dot button (Phosphor); `label` is its screen-reader name.
pub fn dots(ui: &mut egui::Ui, _t: &DesignTokens, label: &str) -> egui::Response {
    crate::icons::button(ui, crate::icons::Icon::DotsThreeVertical, false, label)
}

/// «⋮» of the list header; draws nothing unless a space is selected.
pub fn menu(ui: &mut egui::Ui, session: &mut crate::session::Session, t: &DesignTokens) {
    let Some(scope) = session.sidebar.active_space().cloned() else { return };
    let can_edit = session.sidebar.spaces().any(|r| r.scope == scope && r.info.edit_rights.any());
    let resp = dots(ui, t, i18n::t("Space menu"));
    if resp.clicked() {
        session.space_facts_for(scope.clone());
    }
    let mut popup = egui::Popup::menu(&resp);
    // Test hook: GNOBBI_EGUI_VIEW=sp-menu shows the menu without a click.
    if std::env::var("GNOBBI_EGUI_VIEW").as_deref() == Ok("sp-menu") {
        popup = popup.open(true);
    }
    popup.show(|ui| {
        // Opened by the keyboard or the hook: the click above did not ask.
        if !session.space_facts.as_ref().is_some_and(|(s, _)| *s == scope) {
            session.space_facts_for(scope.clone());
        }
        // Unknown yet (or the engine failed): not offered, no line.
        let block = session.space_facts.as_ref().and_then(|(_, c)| c.as_ref()).map(|c| leave_block(session.sidebar.rooms(), &scope, c));
        if ui.button(i18n::t("Space info")).clicked() {
            session.open_space(scope.clone(), false);
            ui.close();
        }
        if ui.button(i18n::t("Members")).clicked() {
            session.open_space(scope.clone(), true);
            ui.close();
        }
        if can_edit && ui.button(i18n::t("Edit")).clicked() {
            session.open_edit_of(&scope);
            ui.close();
        }
        let facts = session.space_facts.as_ref().and_then(|(_, c)| c.as_ref());
        let (promote, delete) = (facts.is_some_and(can_promote), facts.is_some_and(can_delete));
        if promote && ui.button(i18n::t("Hand over the admin role…")).clicked() {
            session.open_promote(scope.clone());
            ui.close();
        }
        if ui.add_enabled(block == Some(None) && !session.sidebar.space_leaving(), egui::Button::new(i18n::t("Leave space"))).clicked() {
            session.space_exit = Some((scope.clone(), false));
            ui.close();
        }
        if let Some(Some(why)) = block {
            ui.add(egui::Label::new(RichText::new(why.line()).size(12.).color(rgb(t.secondary))).extend());
        }
        if delete && ui.add_enabled(!session.space_deleting, egui::Button::new(RichText::new(i18n::t("Delete space")).color(rgb(t.error)))).clicked() {
            session.space_delete = Some((scope.clone(), String::new()));
            ui.close();
        }
    });
}

/// Confirmation of «Esci dallo spazio»: the name, the «Dimentica» switch, the two buttons.
pub fn exit_window(ctx: &egui::Context, session: &mut crate::session::Session, t: &DesignTokens) {
    let Some((scope, forget)) = &mut session.space_exit else { return };
    // Left or removed meanwhile: nothing to confirm.
    let Some(space) = session.sidebar.spaces().find(|r| r.scope == *scope) else {
        session.space_exit = None;
        return;
    };
    let (mut leave, mut cancel) = (false, false);
    egui::Window::new(i18n::t("Leave space")).id(egui::Id::new("space-exit")).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_width(340.);
        ui.add(egui::Label::new(RichText::new(crate::list::clean_single_line(&space.name)).strong()).truncate());
        ui.add_space(4.);
        crate::settings::switch(ui, i18n::t("Forget"), forget, t);
        ui.add_space(8.);
        ui.horizontal(|ui| {
            leave = ui.button(RichText::new(i18n::t("Leave")).color(rgb(t.error))).clicked();
            cancel = ui.button(i18n::t("Cancel")).clicked();
        });
    });
    if leave || cancel {
        let (scope, forget) = session.space_exit.take().expect("checked above");
        if leave {
            session.leave_space(scope, forget);
        }
    }
}

/// «Elimina spazio»: only the last admin who may edit the space (the engine checks again).
pub fn can_delete(c: &SpaceChildren) -> bool {
    c.last_owner && c.editable
}

/// «Passa il ruolo di amministratore…»: only while the user is the last admin and someone else is in.
pub fn can_promote(c: &SpaceChildren) -> bool {
    c.last_owner && c.other_members > 0
}

/// The typed name has to be the space's own (surrounding blanks ignored, case kept).
pub fn name_confirmed(typed: &str, name: &str) -> bool {
    !name.trim().is_empty() && typed.trim() == name.trim()
}

/// Who may be made admin: joined members except the user. Invited members are not in the engine's list;
/// Matrix has no standard bot flag, so bots cannot be told apart. shortcut: bots listed like people, upgrade if a flag appears.
pub fn admin_candidates<'a>(members: &'a [Member], me: &str) -> Vec<&'a Member> {
    members.iter().filter(|m| m.user_id != me).collect()
}

/// Window of «Elimina spazio»: consequences, the name to type, the red button. Then the report of what was done.
pub fn delete_window(ctx: &egui::Context, session: &mut crate::session::Session, t: &DesignTokens) {
    if let Some((name, result)) = &session.space_report {
        let mut close = false;
        egui::Window::new(i18n::t("Delete space")).id(egui::Id::new("space-report")).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
            ui.set_width(340.);
            ui.add(egui::Label::new(RichText::new(crate::list::clean_single_line(name)).strong()).truncate());
            match result {
                Err(_) => drop(ui.colored_label(rgb(t.error), i18n::t("Failed, no change guaranteed: try again"))),
                Ok(d) => {
                    ui.label(i18n::t("Rooms unlinked: {n}").replace("{n}", &d.unlinked.to_string()));
                    if d.unlink_failed > 0 {
                        ui.colored_label(rgb(t.error), i18n::t("Rooms not unlinked: {n}").replace("{n}", &d.unlink_failed.to_string()));
                    }
                    ui.label(i18n::t("Members removed: {n}").replace("{n}", &d.kicked.to_string()));
                    match &d.kick_failed {
                        None => drop(ui.colored_label(rgb(t.error), i18n::t("Member list not read"))),
                        Some(f) if !f.is_empty() => drop(ui.colored_label(rgb(t.error), i18n::t("Not removed: {names}").replace("{names}", &f.join(", ")))),
                        Some(_) => {}
                    }
                    match &d.left {
                        Some(Ok(_)) => drop(ui.label(i18n::t("Space left and forgotten"))),
                        Some(Err(_)) => drop(ui.colored_label(rgb(t.error), i18n::t("Space not left: try again"))),
                        None => drop(ui.colored_label(rgb(t.error), i18n::t("Space not left: members remain"))),
                    }
                }
            }
            ui.add_space(8.);
            close = ui.button(i18n::t("Close")).clicked();
        });
        if close {
            session.space_report = None;
        }
        return;
    }
    let Some((scope, typed)) = &mut session.space_delete else { return };
    let Some(space) = session.sidebar.spaces().find(|r| r.scope == *scope) else {
        session.space_delete = None;
        return;
    };
    let facts = session.space_facts.as_ref().filter(|(s, _)| s == scope).and_then(|(_, c)| c.as_ref());
    let (rooms, people) = facts.map_or((0, 0), |c| (c.children.len(), c.other_members));
    let (mut go, mut cancel) = (false, false);
    let busy = session.space_deleting;
    egui::Window::new(i18n::t("Delete space")).id(egui::Id::new("space-delete")).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_width(340.);
        ui.add(egui::Label::new(RichText::new(crate::list::clean_single_line(&space.name)).strong()).truncate());
        ui.add_space(4.);
        ui.label(i18n::t("Members who will be removed: {n}").replace("{n}", &people.to_string()));
        ui.label(i18n::t("Rooms unlinked (they stay existing): {n}").replace("{n}", &rooms.to_string()));
        ui.colored_label(rgb(t.error), i18n::t("This cannot be undone"));
        ui.add_space(4.);
        ui.add_enabled(!busy, crate::metrics::line(typed).hint_text(space.name.as_str()).desired_width(f32::INFINITY));
        ui.add_space(8.);
        ui.horizontal(|ui| {
            let red = egui::Button::new(RichText::new(i18n::t("Delete space")).color(egui::Color32::WHITE)).fill(rgb(t.error));
            go = ui.add_enabled(name_confirmed(typed, &space.name) && !busy && facts.is_some(), red).clicked();
            cancel = ui.add_enabled(!busy, egui::Button::new(i18n::t("Cancel"))).clicked();
            if busy {
                ui.spinner();
            }
        });
    });
    let scope = scope.clone();
    if go {
        session.space_delete = None;
        session.delete_space(scope);
    } else if cancel {
        session.space_delete = None;
    }
}

/// Why «Esci dallo spazio» is not offered; `line` is the one line shown, nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveBlock {
    OwnRooms,
    LastAdmin,
}

impl LeaveBlock {
    pub fn line(self) -> &'static str {
        match self {
            Self::OwnRooms => i18n::t("You still have rooms in this space."),
            Self::LastAdmin => i18n::t("You are the last admin of the space."),
        }
    }
}

/// `None` = the user may leave `space`. A space that is no longer in the list is already left: nothing blocks.
/// Child rooms count when the hierarchy says joined, or the list holds them (joined or invited).
pub fn leave_block(rooms: &[SidebarRoom], space: &RoomScope, children: &SpaceChildren) -> Option<LeaveBlock> {
    let info = &rooms.iter().find(|r| &r.scope == space)?.info;
    let own_child = children.children.iter().any(|c| c.joined)
        || rooms.iter().any(|r| r.scope.account_id == space.account_id && info.children.contains(&r.scope.room_id));
    if own_child {
        Some(LeaveBlock::OwnRooms)
    } else if children.last_owner {
        Some(LeaveBlock::LastAdmin)
    } else {
        None
    }
}

/// The leave itself is the engine's `leave_space` (handle, forget, already-left tolerance); the answer is `SpaceLeft`.
pub fn leave_command(scope: RoomScope, forget: bool) -> matrix_engine::operations::OperationCommand {
    matrix_engine::operations::OperationCommand::LeaveSpace { scope, forget }
}

/// Rooms the user is in that can still go under the space: no spaces, no invitations, not already a child.
fn candidates<'a>(rooms: &'a [SidebarRoom], children: &SpaceChildren) -> Vec<&'a SidebarRoom> {
    rooms.iter().filter(|r| !r.info.is_space && !r.info.invited && !children.children.iter().any(|c| c.scope == r.scope)).collect()
}

fn avatar(ui: &mut egui::Ui, avatars: &crate::timeline::Avatars, name: &str, mxc: Option<&str>, t: &DesignTokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.), Sense::hover());
    let initials: String = name.chars().filter(|c| c.is_alphanumeric()).take(2).collect::<String>().to_uppercase();
    crate::avatar::paint(ui, rect.center(), 32., rgb(crate::AVATAR), &crate::avatar::lookup(avatars, mxc), &initials, 12., rgb(t.on_accent));
}

pub fn window(ctx: &egui::Context, session: &mut crate::session::Session, t: &DesignTokens) {
    let Some(view) = &mut session.space_view else { return };
    // Another space picked in the rail, or the space left meanwhile: the window goes.
    let Some(space) = session.sidebar.spaces().find(|r| r.scope == view.scope).filter(|_| session.sidebar.active_space() == Some(&view.scope)) else {
        session.space_view = None;
        return;
    };
    let secondary = rgb(t.secondary);
    let (mut open, mut add, mut remove, mut member, mut promote) = (true, None, None, None, None);
    egui::Window::new(&space.name).id(egui::Id::new("space-view")).open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_width(340.);
        if view.members_page {
            ui.label(RichText::new(i18n::t("Members")).strong());
            match &view.members {
                None => drop(ui.vertical_centered(|ui| ui.spinner())),
                Some(members) => drop(egui::ScrollArea::vertical().max_height(360.).show(ui, |ui| {
                    if view.promote {
                        for m in admin_candidates(members, &session.user) {
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new(crate::list::clean_single_line(&m.name)).truncate());
                                if ui.button(i18n::t("Make admin")).clicked() {
                                    promote = Some(m.user_id.clone());
                                }
                            });
                        }
                        if session.moderation_failed {
                            ui.colored_label(rgb(t.error), i18n::t("Failed"));
                        }
                    } else {
                        member = crate::members::list(ui, members, session.admin.as_ref().filter(|a| a.scope == view.scope), &session.user, &session.avatars, t);
                    }
                })),
            }
            return;
        }
        let children = match &view.children {
            None => {
                ui.vertical_centered(|ui| ui.spinner());
                return;
            }
            Some(Err(())) => {
                ui.colored_label(rgb(t.error), i18n::t("Rooms not available"));
                return;
            }
            Some(Ok(children)) => children,
        };
        if let Some(chosen) = &mut view.adding {
            ui.label(RichText::new(i18n::t("Add existing room")).strong());
            let rooms = candidates(session.sidebar.rooms(), children);
            if rooms.is_empty() {
                ui.label(RichText::new(i18n::t("No rooms")).color(secondary));
            }
            egui::ScrollArea::vertical().max_height(300.).show(ui, |ui| {
                for room in rooms {
                    ui.radio_value(chosen, Some(room.scope.clone()), &room.name);
                }
            });
            ui.add_space(8.);
            let mut cancel = false;
            ui.horizontal(|ui| {
                if ui.add_enabled(chosen.is_some(), egui::Button::new(i18n::t("Add"))).clicked() {
                    add = chosen.take();
                }
                cancel = ui.button(i18n::t("Cancel")).clicked();
            });
            if cancel || add.is_some() {
                view.adding = None;
            }
            return;
        }
        if let Some(topic) = space.info.topic.as_deref().filter(|t| !t.is_empty()) {
            ui.label(RichText::new(topic).color(secondary));
        }
        ui.label(RichText::new(i18n::plural(space.info.joined_members as u64, "{n} member", "{n} members")).color(secondary));
        ui.separator();
        ui.label(RichText::new(i18n::t("Rooms")).strong());
        if children.children.is_empty() {
            ui.label(RichText::new(i18n::t("No rooms")).color(secondary));
        }
        egui::ScrollArea::vertical().max_height(300.).show(ui, |ui| {
            for child in &children.children {
                ui.horizontal(|ui| {
                    avatar(ui, &session.avatars, &child.name, child.avatar_url.as_deref(), t);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if children.editable && ui.button(i18n::t("Remove")).clicked() {
                            remove = Some(child.scope.clone());
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            // A room the user is not in is listed, greyed.
                            let name = RichText::new(crate::list::clean_single_line(&child.name)).color(if child.joined { rgb(t.text) } else { secondary });
                            ui.add(egui::Label::new(name).truncate());
                        });
                    });
                });
            }
        });
        if children.editable {
            ui.add_space(8.);
            if ui.button(i18n::t("Add existing room")).clicked() {
                view.adding = Some(None);
            }
        }
    });
    let space = view.scope.clone();
    if let Some(user) = promote {
        session.moderation_failed = false;
        session.moderate_in(space.clone(), user, matrix_engine::operations::Moderation::SetPower(100));
    }
    if !open {
        session.space_view = None;
    }
    // A row click opens nothing here: the profile card belongs to the room panel.
    if let Some(act) = member.filter(|a| !matches!(a, crate::members::Act::Profile(_))) {
        session.member_act(act);
    }
    if let Some(room) = add {
        session.room_action(room, crate::session::RoomAction::AddTo(space));
    } else if let Some(room) = remove {
        session.room_action(room, crate::session::RoomAction::RemoveFrom(space));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn delete_and_promote_follow_the_last_admin_and_the_other_members() {
        use matrix_engine::operations::{Member, SpaceChildren};
        let c = |last_owner, editable, other_members| SpaceChildren { last_owner, editable, other_members, ..Default::default() };
        assert!(super::can_delete(&c(true, true, 0)) && !super::can_delete(&c(true, false, 3)) && !super::can_delete(&c(false, true, 3)));
        assert!(super::can_promote(&c(true, true, 1)) && !super::can_promote(&c(true, true, 0)) && !super::can_promote(&c(false, true, 5)));
        assert!(super::name_confirmed("  Casa ", "Casa") && !super::name_confirmed("casa", "Casa") && !super::name_confirmed("", "") && !super::name_confirmed("Cas", "Casa"));
        let m = |id: &str| Member { user_id: id.into(), name: id.into(), avatar: None, role: matrix_domain::role::MemberRole::Member };
        let list = [m("@me:x"), m("@ada:x"), m("@bob:x")];
        let ids: Vec<_> = super::admin_candidates(&list, "@me:x").iter().map(|m| m.user_id.as_str()).collect();
        assert_eq!(ids, ["@ada:x", "@bob:x"]);
    }

    #[test]
    fn only_joined_rooms_outside_the_space_can_be_added() {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        use matrix_engine::operations::{SpaceChild, SpaceChildren};
        let scope = |id: &str| RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(id) };
        let room = |id: &str, is_space, invited| matrix_desktop::SidebarRoom {
            scope: scope(id),
            name: id.to_owned(),
            initials: String::new(),
            preview: String::new(),
            time: "",
            color: 0,
            unread: 0,
            info: matrix_engine::RoomInfo { is_space, invited, ..Default::default() },
        };
        let rooms = [room("!free:x", false, false), room("!child:x", false, false), room("!space:x", true, false), room("!invite:x", false, true)];
        let children = SpaceChildren { children: vec![SpaceChild { scope: scope("!child:x"), name: "c".into(), avatar_url: None, joined: true }], editable: true, last_owner: false, other_members: 0 };
        assert_eq!(super::candidates(&rooms, &children).iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["!free:x"]);
    }

    #[test]
    fn leave_eligibility() {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        use matrix_engine::operations::{SpaceChild, SpaceChildren};
        let scope = |id: &str| RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(id) };
        let room = |id: &str, is_space, invited, children: &[&str]| matrix_desktop::SidebarRoom {
            scope: scope(id),
            name: id.to_owned(),
            initials: String::new(),
            preview: String::new(),
            time: "",
            color: 0,
            unread: 0,
            info: matrix_engine::RoomInfo { is_space, invited, children: children.iter().map(|c| RoomId::new(*c)).collect(), ..Default::default() },
        };
        let none = SpaceChildren::default();
        let space = scope("!s:x");
        // No children of the user's: free to leave.
        let rooms = [room("!s:x", true, false, &["!c:x"]), room("!other:x", false, false, &[])];
        assert_eq!(super::leave_block(&rooms, &space, &none), None);
        // Child in the list (joined or invited), or joined per the hierarchy.
        let with_child = [room("!s:x", true, false, &["!c:x"]), room("!c:x", false, true, &[])];
        assert_eq!(super::leave_block(&with_child, &space, &none), Some(super::LeaveBlock::OwnRooms));
        let hierarchy = SpaceChildren { children: vec![SpaceChild { scope: scope("!c:x"), name: "c".into(), avatar_url: None, joined: true }], ..Default::default() };
        assert_eq!(super::leave_block(&rooms, &space, &hierarchy), Some(super::LeaveBlock::OwnRooms));
        // Last admin, only once the rooms are out of the way.
        let last = SpaceChildren { last_owner: true, ..Default::default() };
        assert_eq!(super::leave_block(&rooms, &space, &last), Some(super::LeaveBlock::LastAdmin));
        assert_eq!(super::leave_block(&with_child, &space, &last), Some(super::LeaveBlock::OwnRooms));
        // Already left: the space is gone from the list.
        assert_eq!(super::leave_block(&[room("!other:x", false, false, &[])], &space, &last), None);
        assert!(super::LeaveBlock::LastAdmin.line().len() > 0);
    }
}
