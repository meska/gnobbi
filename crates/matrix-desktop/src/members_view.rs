//! Member list and another user's profile, inside the room sheet. The engine reads both (SDK).
use crate::*;
use matrix_desktop::ProfileAct;
use matrix_engine::operations::{OperationCommand, OperationError, UserProfile};

pub(crate) enum ProfileState {
    Loading(String),
    Loaded(UserProfile),
    Failed(String),
}

fn initial(name: &str) -> String {
    name.trim_start_matches('@')
        .chars()
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().collect())
}

/// Focusable controls of an opened profile, in Tab order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileCtl {
    Back,
    Close,
    Act(ProfileAct),
}

impl Shell {
    fn own_user_id(&self) -> Option<&str> {
        self.accounts.get(self.active_id?)?.user_id.as_deref()
    }

    /// Actions that make sense for the opened profile; future sections (verify, moderation, ignore) extend this
    /// list, destructive ones last.
    pub(crate) fn profile_acts(&self) -> Vec<ProfileAct> {
        let Some(ProfileState::Loaded(p)) = &self.profile else {
            return Vec::new();
        };
        let me = self.own_user_id() == Some(p.user_id.as_str());
        let mut acts = vec![ProfileAct::CopyId];
        if !me {
            acts.push(ProfileAct::Message);
        }
        acts.push(ProfileAct::Link);
        if !me {
            acts.push(ProfileAct::Quote);
        }
        acts
    }

    fn profile_ctls(&self) -> Vec<ProfileCtl> {
        [ProfileCtl::Back, ProfileCtl::Close]
            .into_iter()
            .chain(self.profile_acts().into_iter().map(ProfileCtl::Act))
            .collect()
    }

    fn activate_ctl(&mut self, ctl: ProfileCtl, window: &mut Window, cx: &mut Context<Self>) {
        match ctl {
            ProfileCtl::Back => self.profile = None,
            ProfileCtl::Close => self.set_room_details_open(false),
            ProfileCtl::Act(act) => self.profile_act(act, window, cx),
        }
        cx.notify();
    }

    /// Keys of an opened profile; true when consumed. Esc goes back to the list (the list's own Esc closes the sheet).
    pub(crate) fn profile_key(&mut self, key: &str, shift: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let ctls = self.profile_ctls();
        match key {
            "escape" => self.profile = None,
            "tab" => {
                let n = ctls.len();
                self.profile_focus = (self.profile_focus + if shift { n - 1 } else { 1 }) % n;
            }
            "enter" | "space" => {
                if let Some(&ctl) = ctls.get(self.profile_focus) {
                    self.activate_ctl(ctl, window, cx);
                }
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(crate) fn profile_act(&mut self, act: ProfileAct, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ProfileState::Loaded(p)) = &self.profile else {
            return;
        };
        let (user_id, name) = (p.user_id.clone(), p.name.clone());
        match act {
            ProfileAct::CopyId | ProfileAct::Link => {
                let text = if act == ProfileAct::CopyId { user_id } else { format!("https://matrix.to/#/{user_id}") };
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                self.profile_copied = Some(act);
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(1200)).await;
                    this.update(cx, |this, cx| {
                        this.profile_copied = None;
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            ProfileAct::Message => {
                if self.pending_operation_command.is_none() {
                    self.pending_operation_command = Some(OperationCommand::StartDm { user_id });
                    self.profile_dm = true;
                }
            }
            ProfileAct::Quote => {
                self.set_room_details_open(false);
                self.composer.update(cx, |editor, cx| editor.insert_mention(user_id, &name, window, cx));
            }
        }
        cx.notify();
    }

    /// Row click: ask the engine for the profile; a click while another command waits is ignored.
    pub(crate) fn open_profile(&mut self, user_id: String, cx: &mut Context<Self>) {
        if self.pending_operation_command.is_none() {
            self.pending_operation_command = Some(OperationCommand::Profile {
                user_id: user_id.clone(),
            });
            self.profile = Some(ProfileState::Loading(user_id));
            self.profile_focus = 0;
            self.profile_copied = None;
            cx.notify();
        }
    }

    pub(crate) fn profile_found(
        &mut self,
        user_id: String,
        result: Result<UserProfile, OperationError>,
        cx: &mut Context<Self>,
    ) {
        let waiting = matches!(&self.profile, Some(ProfileState::Loading(id)) if *id == user_id);
        if waiting {
            self.profile = Some(match result {
                Ok(profile) => ProfileState::Loaded(profile),
                Err(_) => ProfileState::Failed(user_id),
            });
            cx.notify();
        }
    }

    /// Avatars the open sheet shows.
    pub(crate) fn people_avatars(&self) -> Vec<String> {
        if !self.room_details_open {
            return Vec::new();
        }
        match &self.profile {
            Some(ProfileState::Loaded(p)) => p.avatar.clone().into_iter().collect(),
            Some(_) => Vec::new(),
            None => self.room_members.iter().filter_map(|m| m.avatar.clone()).collect(),
        }
    }

    /// Screen-reader labels of what the sheet lists below the room data.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn people_labels(&self) -> Vec<(String, Option<String>)> {
        match &self.profile {
            Some(ProfileState::Loaded(p)) => vec![(format!("Profilo: {}, {}", p.name, p.user_id), None)],
            Some(_) => Vec::new(),
            None => self
                .room_members
                .iter()
                .map(|m| (format!("Membro: {}, {}", m.name, m.user_id), Some(m.user_id.clone())))
                .collect(),
        }
    }

    fn picture(&self, name: &str, mxc: Option<&str>, size: f32, color: u32, text: u32) -> gpui::AnyElement {
        match self.avatars.get(mxc) {
            Some(image) => avatar_view::round(image, size),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(size))
                .flex_shrink_0()
                .rounded_full()
                .bg(rgb(color))
                .text_color(rgb(text))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(initial(name))
                .into_any_element(),
        }
    }

    pub(crate) fn render_people(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        let back = |cx: &mut Context<Self>| {
            div()
                .id("profile-back")
                .cursor_pointer()
                .text_color(rgb(t.accent))
                .child("Membri")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.profile = None;
                    cx.notify();
                }))
        };
        match &self.profile {
            Some(ProfileState::Loaded(p)) => {
                let ctls = self.profile_ctls();
                let ring = |ctl: ProfileCtl| {
                    let on = ctls.get(self.profile_focus) == Some(&ctl);
                    move |d: gpui::Stateful<gpui::Div>| d.border_1().border_color(rgb(if on { t.accent } else { t.surface }))
                };
                let icon_button = |id: &'static str, icon: composer_icons::Icon, ctl: ProfileCtl, color: u32, cx: &mut Context<Self>| {
                    ring(ctl)(div().id(id))
                        .p_1()
                        .cursor_pointer()
                        .child(composer_icons::icon(icon, color))
                        .on_click(cx.listener(move |this, _, window, cx| this.activate_ctl(ctl, window, cx)))
                };
                // One separated block per kind of action; verify, moderation and ignore are added as further blocks,
                // the destructive ones at the end.
                let section = || div().flex().flex_col().w_full().pt_2().border_t_1().border_color(rgb(t.border));
                let acts = self.profile_acts();
                let copied = self.profile_copied;
                div()
                    .id("profile-panel")
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .justify_between()
                            .child(icon_button("profile-back", composer_icons::Icon::Back, ProfileCtl::Back, t.accent, cx))
                            .child(icon_button("profile-close", composer_icons::Icon::Close, ProfileCtl::Close, t.accent, cx)),
                    )
                    .child(self.picture(&p.name, p.avatar.as_deref(), 120., t.accent, t.avatar_text))
                    .child(
                        div()
                            .max_w_full()
                            .text_size(px(20.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(matrix_desktop::row_text::one_line(&p.name)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .max_w_full()
                            .child(div().truncate().text_size(px(12.)).text_color(rgb(t.secondary)).child(p.user_id.clone()))
                            .child(icon_button(
                                "profile-copy-id",
                                composer_icons::Icon::Copy,
                                ProfileCtl::Act(ProfileAct::CopyId),
                                if copied == Some(ProfileAct::CopyId) { t.accent } else { t.secondary },
                                cx,
                            )),
                    )
                    .child(section().children(
                        acts.into_iter().filter(|a| *a != ProfileAct::CopyId).map(|act| {
                            let ctl = ProfileCtl::Act(act);
                            ring(ctl)(div().id(act.label()))
                                .h(px(40.))
                                .flex()
                                .items_center()
                                .px_2()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(t.border)))
                                .text_color(rgb(if copied == Some(act) { t.accent } else { t.text }))
                                .child(act.label())
                                .on_click(cx.listener(move |this, _, window, cx| this.activate_ctl(ctl, window, cx)))
                        }),
                    ))
                    .into_any_element()
            }
            Some(state) => {
                let (ProfileState::Loading(id) | ProfileState::Failed(id)) = state else {
                    unreachable!()
                };
                div()
                    .id("profile-panel")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(back(cx))
                    .child(div().truncate().child(id.clone()))
                    .when(matches!(state, ProfileState::Failed(_)), |d| {
                        d.child(div().text_color(rgb(t.error)).child("Profilo non disponibile"))
                    })
                    .into_any_element()
            }
            None => div()
                .id("member-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(176.)) // 4 rows even in a short window; the info block scrolls instead
                .child(div().mb_2().child("Membri"))
                .child(
                    // Same virtual list + scrollbar as the room list: only visible rows exist.
                    div().relative().flex_1().min_h_0().child(
                        uniform_list(
                            "members",
                            self.room_members.len(),
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                let t = this.theme.tokens();
                                range
                                    .map(|i| {
                                        let m = &this.room_members[i];
                                        let id = m.user_id.clone();
                                        div()
                                            .id(("member", i))
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .h(px(44.))
                                            .flex_shrink_0()
                                            .cursor_pointer()
                                            .hover(|s| s.bg(rgb(t.border)))
                                            .child(this.picture(&m.name, m.avatar.as_deref(), 32., t.accent, t.avatar_text))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .min_w_0()
                                                    .child(div().truncate().child(matrix_desktop::row_text::one_line(&m.name)))
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_size(px(12.))
                                                            .text_color(rgb(t.secondary))
                                                            .child(m.user_id.clone()),
                                                    ),
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| this.open_profile(id.clone(), cx)))
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(self.members_scroll.clone())
                        .size_full(),
                    )
                    .child(self.members_scrollbar.render(
                        "members-scrollbar",
                        scrollbar_view::ScrollTarget::Rooms(self.members_scroll.clone()),
                        t,
                    )),
                )
                .into_any_element(),
        }
    }
}
