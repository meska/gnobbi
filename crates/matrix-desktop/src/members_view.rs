//! Member list and another user's profile, inside the room sheet. The engine reads both (SDK).
use crate::*;
use matrix_engine::operations::{Member, OperationCommand, OperationError, UserProfile};

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

impl Shell {
    /// Row click: ask the engine for the profile; a click while another command waits is ignored.
    pub(crate) fn open_profile(&mut self, user_id: String, cx: &mut Context<Self>) {
        if self.pending_operation_command.is_none() {
            self.pending_operation_command = Some(OperationCommand::Profile {
                user_id: user_id.clone(),
            });
            self.profile = Some(ProfileState::Loading(user_id));
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
            Some(ProfileState::Loaded(p)) => div()
                .id("profile-panel")
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .child(div().w_full().child(back(cx)))
                .child(self.picture(&p.name, p.avatar.as_deref(), 96., t.accent, t.avatar_text))
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(matrix_desktop::row_text::one_line(&p.name)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(t.secondary))
                        .child(p.user_id.clone()),
                )
                .into_any_element(),
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
                .child(div().mb_2().child("Membri"))
                .children(self.room_members.iter().enumerate().map(|(i, m): (usize, &Member)| {
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
                        .child(self.picture(&m.name, m.avatar.as_deref(), 32., t.accent, t.avatar_text))
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
                }))
                .into_any_element(),
        }
    }
}
