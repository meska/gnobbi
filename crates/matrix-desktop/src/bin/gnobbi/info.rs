//! Right panel: the open room's facts and its members.
use crate::list::rgb;
use eframe::egui::{self, Color32, FontId, RichText, Vec2};
use matrix_desktop::{DesignTokens, SidebarRoom, i18n};
use matrix_engine::operations::Member;

/// What the user did in the panel this frame.
#[derive(Default)]
pub struct Out {
    pub member: Option<crate::members::Act>,
    pub open_link: Option<matrix_desktop::SafeWebUrl>,
    pub edit: bool,
}

pub fn show(ui: &mut egui::Ui, room: &SidebarRoom, members: Option<&[Member]>, admin: Option<&crate::members::Admin>, me: &str, aliases: &mut AliasView, (status, hidden_status): (&mut matrix_desktop::StatusChoice, usize), avatars: &crate::avatar::Avatars, t: &DesignTokens) -> Out {
    let mut out = Out::default();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(16.);
        ui.horizontal(|ui| {
            ui.add_space(16.);
            ui.with_layout(crate::dir::col(), |ui| {
                ui.set_max_width(ui.available_width() - 16.);
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - if room.info.edit_rights.any() { crate::metrics::button() + ui.spacing().item_spacing.x } else { 0. }).max(0.);
                    ui.add_sized([width, crate::metrics::button()], egui::Label::new(RichText::new(&room.name).heading()).truncate());
                    if room.info.edit_rights.any() && crate::icons::button(ui, crate::icons::Icon::PencilSimple, false, i18n::t("Edit")).on_hover_text(i18n::t("Edit")).clicked() {
                        out.edit = true;
                    }
                });
                if let Some(topic) = room.info.shown_topic().filter(|t| !t.text.is_empty()) {
                    ui.add_space(4.);
                    out.open_link = topic_view(ui, &topic, t);
                }
                ui.add_space(8.);
                ui.label(RichText::new(i18n::plural(room.info.joined_members as u64, "{n} member", "{n} members")).color(rgb(t.secondary)));
                if room.info.encrypted == Some(true) {
                    ui.label(RichText::new(i18n::t("Encrypted")).color(rgb(t.secondary)));
                }
            });
        });
        ui.add_space(12.);
        addresses(ui, aliases, t);
        ui.add_space(8.);
        ui.horizontal(|ui| {
            ui.add_space(16.);
            ui.with_layout(crate::dir::col(), |ui| {
                ui.set_max_width(ui.available_width() - 16.);
                let label = i18n::t("Status messages");
                let name = ui.label(label);
                crate::metrics::clickable(egui::ComboBox::from_id_salt(label).selected_text(status.label()).width(ui.available_width()).show_ui(ui, |ui| {
                    for c in matrix_desktop::StatusChoice::ALL {
                        ui.selectable_value(status, c, c.label());
                    }
                }).response).labelled_by(name.id);
                if hidden_status > 0 {
                    ui.label(RichText::new(i18n::plural(hidden_status as u64, "{n} status message hidden", "{n} status messages hidden")).size(11.).color(rgb(t.secondary)));
                }
            });
        });
        let Some(members) = members else {
            ui.add_space(12.);
            ui.vertical_centered(|ui| ui.spinner());
            return;
        };
        out.member = crate::members::list(ui, members, admin, me, avatars, t);
    });
    out
}

/// One stretch of the topic drawn in a single style; `url` makes it a link.
struct Run<'a> {
    text: &'a str,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    url: Option<matrix_desktop::SafeWebUrl>,
}

/// The engine's topic cut at every style and link edge; bare URLs become links as in the timeline, unsafe hrefs stay text.
fn topic_runs(topic: &matrix_engine::rich_text::Topic) -> Vec<Run<'_>> {
    use matrix_engine::rich_text::Style;
    let text = topic.text.as_str();
    let mut cuts = vec![0, text.len()];
    cuts.extend(topic.styles.iter().flat_map(|s| [s.range.start, s.range.end]));
    cuts.extend(topic.links.iter().flat_map(|l| [l.range.start, l.range.end]));
    cuts.retain(|&c| text.is_char_boundary(c));
    cuts.sort_unstable();
    cuts.dedup();
    let mut runs = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let has = |style| topic.styles.iter().any(|s| s.style == style && s.range.start <= a && b <= s.range.end);
        let href = topic.links.iter().find(|l| l.range.start <= a && b <= l.range.end).and_then(|l| matrix_desktop::SafeWebUrl::parse(&l.href));
        let parts = match href {
            Some(url) => vec![(&text[a..b], Some(url))],
            None => crate::timeline::link_parts(&text[a..b]),
        };
        let (bold, italic, strike, code) = (has(Style::Bold), has(Style::Italic), has(Style::Strike), has(Style::Code));
        runs.extend(parts.into_iter().map(|(text, url)| Run { text, bold, italic, strike, code, url }));
    }
    runs
}

/// The topic with its styles; links look and behave as in the timeline. Returns the link clicked this frame.
fn topic_view(ui: &mut egui::Ui, topic: &matrix_engine::rich_text::Topic, t: &DesignTokens) -> Option<matrix_desktop::SafeWebUrl> {
    let mut clicked = None;
    crate::dir::wrapped(ui, crate::dir::content_rtl(&topic.text), |ui| {
        ui.spacing_mut().item_spacing.x = 0.;
        for run in topic_runs(topic) {
            // shortcut: no bold face is bundled, bold is the main text colour; add a bold font to upgrade.
            let mut text = RichText::new(run.text).color(rgb(if run.url.is_some() { t.accent } else if run.bold { t.text } else { t.secondary }));
            if run.italic {
                text = text.italics();
            }
            if run.strike {
                text = text.strikethrough();
            }
            if run.code {
                text = text.monospace();
            }
            match run.url {
                Some(url) => {
                    if ui.link(text.underline()).clicked() {
                        clicked = Some(url);
                    }
                }
                None => {
                    ui.label(text);
                }
            }
        }
    });
    clicked
}

#[cfg(test)]
mod topic_tests {
    #[test]
    fn alias_row_wraps_before_the_server_or_field_is_squeezed() {
        assert!(!super::alias_wrap(280., 100., 30., 8.));
        assert!(super::alias_wrap(200., 100., 30., 8.));
        assert!(super::alias_wrap(0., 100., 30., 8.));
    }

    #[test]
    fn duplicate_member_row_opens_on_click() {
        use eframe::egui::{self, Event, Modifiers, PointerButton};
        let ctx = egui::Context::default();
        let m = matrix_engine::operations::Member { user_id: "@one:example.org".into(), name: "Marco".into(), avatar: None, role: matrix_domain::role::MemberRole::Admin };
        let mut rect = egui::Rect::NOTHING;
        let mut clicked = false;
        for step in 0..4 {
            let pos = rect.center();
            let events = match step {
                2 => vec![Event::PointerMoved(pos), Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }],
                3 => vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 200.))), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = super::member_row(ui, &Default::default(), &m, true, &matrix_desktop::LIGHT);
                    rect = r.rect;
                    clicked |= r.clicked();
                });
            });
            output.textures_delta.clear();
        }
        assert!(clicked);
    }

    #[test]
    fn topic_runs_carry_styles_links_and_lists() {
        let topic = matrix_engine::rich_text::topic("[sito](https://example.org) **forte** *chino* [no](javascript:alert(1)) https://nudo.example\n\n- uno\n- due\n\n1. primo", None);
        let runs = super::topic_runs(&topic);
        assert_eq!(runs.iter().map(|r| r.text).collect::<String>(), topic.text);
        let run = |s: &str| runs.iter().find(|r| r.text.contains(s)).unwrap();
        assert!(run("sito").text == "sito" && run("sito").url.as_ref().is_some_and(|u| u.as_str().starts_with("https://example.org")));
        assert!(run("forte").bold && !run("forte").italic && run("chino").italic && !run("chino").bold);
        assert!(runs.iter().all(|r| r.url.as_ref().is_none_or(|u| !u.as_str().starts_with("javascript"))));
        assert!(run("nudo").url.is_some());
        assert!(topic.text.contains("• uno") && topic.text.contains("1. primo") && !topic.text.contains("**"));
        let plain = matrix_engine::RoomInfo { topic: Some("Solo testo è".into()), ..Default::default() }.shown_topic().unwrap();
        let runs = super::topic_runs(&plain);
        assert!(runs.len() == 1 && runs[0].text == "Solo testo è" && runs[0].url.is_none() && !runs[0].bold);
    }

    #[test]
    fn topic_runs_hide_raw_markdown_and_keep_plain_and_empty_unchanged() {
        let topic = matrix_engine::rich_text::topic("[sito](https://example.org) **forte** *chino*\n\n- uno\n- due\n\n1. primo\n2. secondo", None);
        let runs = super::topic_runs(&topic);
        let shown: String = runs.iter().map(|r| r.text).collect();
        for raw in ["**", "*", "](", "[sito]"] {
            assert!(!shown.contains(raw), "markdown grezzo {raw:?} in {shown:?}");
        }
        assert!(shown.contains("• uno") && shown.contains("• due") && shown.contains("1. primo") && shown.contains("2. secondo"));
        assert!(runs.iter().any(|r| r.text == "sito" && r.url.is_some()));
        assert!(runs.iter().any(|r| r.text == "forte" && r.bold) && runs.iter().any(|r| r.text == "chino" && r.italic));
        // Plain topic: one run, same text, no style (the old label's content).
        let plain = matrix_engine::RoomInfo { topic: Some("Benvenuti nella stanza".into()), ..Default::default() };
        let shown_plain = plain.shown_topic().unwrap();
        let runs = super::topic_runs(&shown_plain);
        assert!(runs.len() == 1 && runs[0].text == "Benvenuti nella stanza" && runs[0].url.is_none() && !runs[0].bold && !runs[0].italic);
        // Empty topic: nothing to draw (info::show filters it out) and no runs.
        assert!(matrix_engine::RoomInfo::default().shown_topic().is_none());
        assert!(super::topic_runs(&matrix_engine::rich_text::topic("", None)).is_empty());
    }
}

/// One member: initials, name on one line and the role badge at the right edge.
pub fn member_row(ui: &mut egui::Ui, avatars: &crate::avatar::Avatars, m: &Member, duplicate: bool, t: &DesignTokens) -> egui::Response {
    use matrix_domain::role::MemberRole;
    if duplicate {
        let role = crate::members::role_label(m.role);
        let name = crate::list::clean_single_line(&m.name);
        let id = crate::list::clean_single_line(&m.user_id);
        let row = ui.scope(|ui| {
            crate::rows::lines_row(ui, |ui| {
                ui.add_space(14.);
                let (r, _) = ui.allocate_exact_size(Vec2::splat(28.), egui::Sense::hover());
                crate::avatar::paint(ui, r.center(), 28., rgb(matrix_desktop::AVATAR_FALLBACK), &crate::avatar::lookup(avatars, m.avatar.as_deref()), &matrix_desktop::initials(&m.name), 12., rgb(t.on_accent));
            }, &[crate::rows::Line::new(&name, 14., rgb(t.text)), crate::rows::Line::new(&id, 12., rgb(t.secondary))], |ui| {
                let ink = rgb(match m.role { MemberRole::Admin => t.on_accent, MemberRole::Moderator => t.accent, MemberRole::Member => t.secondary });
                let label = ui.painter().layout_no_wrap(role.to_owned(), FontId::proportional(11.), ink);
                let (pill, _) = ui.allocate_exact_size(Vec2::new(label.size().x + 16., 20.), egui::Sense::hover());
                match m.role {
                    MemberRole::Admin => ui.painter().rect_filled(pill, 10., rgb(t.accent)),
                    MemberRole::Moderator => ui.painter().rect_stroke(pill, 10., egui::Stroke::new(1., rgb(t.accent)), egui::StrokeKind::Inside),
                    MemberRole::Member => ui.painter().rect_stroke(pill, 10., egui::Stroke::new(1., rgb(t.border)), egui::StrokeKind::Inside),
                };
                ui.painter().galley(pill.center() - label.size() / 2., label, ink);
                ui.add_space(8.);
            });
        }).response;
        let resp = crate::metrics::clickable(ui.interact(row.rect, row.id, egui::Sense::click()));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{}, {}, {role}", m.name, m.user_id)));
        return resp;
    }
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.), egui::Sense::click());
    let resp = crate::metrics::clickable(resp);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0., rgb(t.input));
    }
    let c = crate::dir::flip_pos(rect, egui::pos2(rect.left() + 28., rect.center().y));
    let initials: String = m.name.trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
    crate::avatar::paint(ui, c, 28., rgb(matrix_desktop::AVATAR_FALLBACK), &crate::avatar::lookup(avatars, m.avatar.as_deref()), &initials, 12., rgb(t.on_accent));
    let role = crate::members::role_label(m.role);
    let ink = rgb(match m.role {
        MemberRole::Admin => t.on_accent,
        MemberRole::Moderator => t.accent,
        MemberRole::Member => t.secondary,
    });
    let label = ui.painter().layout_no_wrap(role.to_owned(), FontId::proportional(11.), ink);
    let pill = egui::Rect::from_center_size(egui::pos2(rect.right() - 8. - (label.size().x + 16.) / 2., rect.center().y), Vec2::new(label.size().x + 16., 20.));
    // `pill` stays the LTR one (the name's width is measured against it); `shown` is where it is painted.
    let shown = crate::dir::flip(rect, pill);
    match m.role {
        MemberRole::Admin => ui.painter().rect_filled(shown, 10., rgb(t.accent)),
        MemberRole::Moderator => ui.painter().rect_stroke(shown, 10., egui::Stroke::new(1., rgb(t.accent)), egui::StrokeKind::Inside),
        MemberRole::Member => ui.painter().rect_stroke(shown, 10., egui::Stroke::new(1., rgb(t.border)), egui::StrokeKind::Inside),
    };
    ui.painter().galley(shown.center() - label.size() / 2., label, ink);
    let name = crate::list::one_line(ui, &m.name, 14., rgb(t.text), (pill.left() - 8. - rect.left() - 52.).max(20.));
    ui.painter().galley(crate::dir::place(rect, egui::pos2(rect.left() + 52., rect.center().y - name.size().y / 2.), name.size().x), name, rgb(t.text));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{}, {role}", m.name)));
    resp
}

/// Message search of the open room.
pub fn search(ui: &mut egui::Ui, term: &mut String, state: &matrix_engine::operations::MessageSearchState, more_clicked: &mut bool, t: &DesignTokens) -> Option<(matrix_domain::EventId, bool)> {
    let mut selected = None;
    use matrix_engine::operations::MessageSearchState as S;
    ui.add_space(12.);
    ui.horizontal(|ui| {
        ui.add_space(12.);
        let w = ui.available_width() - 12.;
        crate::metrics::search(ui, [w, crate::metrics::button()], term, i18n::t("Search in room"));
    });
    ui.add_space(8.);
    match state {
        S::Idle => {}
        S::Searching { .. } => {
            ui.vertical_centered(|ui| ui.spinner());
        }
        S::NoResults { .. } => {
            ui.label(RichText::new(i18n::t("No results")).color(rgb(t.secondary)));
        }
        S::Error { .. } => {
            ui.colored_label(rgb(t.error), i18n::t("Search failed"));
        }
        S::Results { hits, more, .. } => {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for h in hits {
                    ui.add_space(6.);
                    let response = ui.horizontal(|ui| {
                        ui.add_space(12.);
                        ui.with_layout(crate::dir::col(), |ui| {
                            ui.set_max_width(ui.available_width() - 12.);
                            ui.label(RichText::new(&h.sender_name).strong().size(12.).color(rgb(t.accent)));
                            ui.label(&h.excerpt);
                        });
                    }).response.interact(egui::Sense::click());
                    let response = crate::metrics::clickable(response);
                    if response.clicked() {
                        selected = Some((matrix_domain::EventId::new(h.thread_root.as_deref().unwrap_or(&h.event_id)), h.thread_root.is_some()));
                    }
                    ui.separator();
                }
                match more {
                    matrix_engine::operations::MoreHits::Available | matrix_engine::operations::MoreHits::Failed => {
                        let label = if matches!(more, matrix_engine::operations::MoreHits::Failed) { i18n::t("Retry") } else { i18n::t("More results") };
                        if ui.button(label).clicked() { *more_clicked = true; }
                    }
                    matrix_engine::operations::MoreHits::Loading => { ui.spinner(); }
                    _ => {}
                }
            });
        }
    }
    selected
}

#[derive(Clone, Copy)]
pub enum ProfileAction { Message, Mention, Verify, ReadReceipt }

/// A member's card; returns the new ignore state when flipped.
pub fn card(ui: &mut egui::Ui, id: &str, profile: Option<&matrix_engine::operations::UserProfile>, rights: Option<&matrix_engine::operations::MemberRights>, failed: bool, busy: bool, dm_failed: bool, me: bool, can_verify: bool, action: &mut Option<ProfileAction>, confirm: &mut Option<&'static str>, moderation: &mut Option<matrix_engine::operations::Moderation>, close: &mut bool, t: &DesignTokens) -> Option<bool> {
    let mut flip = None;
    ui.add_space(16.);
    ui.horizontal(|ui| {
        ui.add_space(16.);
        if crate::info::close_icon(ui, i18n::t("Close card")).clicked() {
            *close = true;
        }
    });
    let Some(p) = profile else {
        ui.add_space(16.);
        ui.vertical_centered(|ui| ui.spinner());
        return None;
    };
    ui.horizontal(|ui| {
        ui.add_space(16.);
        ui.with_layout(crate::dir::col(), |ui| {
            ui.set_max_width(ui.available_width() - 16.);
            ui.heading(&p.name);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(id).color(rgb(t.secondary)));
                if copy_icon(ui).clicked() {
                    ui.ctx().copy_text(id.to_owned());
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(i18n::t("Profile link"));
                if copy_icon(ui).clicked() {
                    ui.ctx().copy_text(format!("https://matrix.to/#/{id}"));
                }
            });
            match p.verified {
                Some(true) => ui.label(i18n::t("Verified")),
                Some(false) => ui.label(RichText::new(i18n::t("Not verified")).color(rgb(t.error))),
                None => ui.label(""),
            };
            if !me {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(i18n::t("Message")).clicked() { *action = Some(ProfileAction::Message); }
                    if ui.button(i18n::t("Mention")).clicked() { *action = Some(ProfileAction::Mention); }
                    if p.verified == Some(false) && ui.add_enabled(can_verify, egui::Button::new(i18n::t("Verify"))).clicked() { *action = Some(ProfileAction::Verify); }
                    if rights.is_some_and(|r| r.read_receipt.is_some()) && ui.button(i18n::t("Last read")).clicked() { *action = Some(ProfileAction::ReadReceipt); }
                });
                if dm_failed { ui.colored_label(rgb(t.error), i18n::t("Chat unavailable")); }
            }
            ui.add_space(8.);
            let mut ignored = p.ignored;
            if crate::settings::switch(ui, i18n::t("Ignore"), &mut ignored, t) {
                flip = Some(ignored);
            }
            if let Some(r) = rights {
                use matrix_domain::role::MemberRole;
                use matrix_engine::operations::Moderation;
                ui.add_space(8.);
                ui.separator();
                // The role: plain text for the creator and without the power level, else only the roles the user may give.
                let roles: Vec<_> = [MemberRole::Member, MemberRole::Moderator, MemberRole::Admin].into_iter().filter(|x| *x == r.role || (!r.creator && r.levels.contains(&x.power_level()))).collect();
                ui.horizontal(|ui| {
                    if busy {
                        ui.spinner();
                    } else if roles.len() < 2 {
                        ui.label(crate::members::role_label(r.role));
                    } else {
                        for role in roles {
                            if ui.selectable_label(role == r.role, crate::members::role_label(role)).clicked() && role != r.role {
                                *moderation = Some(Moderation::SetPower(role.power_level()));
                            }
                        }
                    }
                });
                // Each destructive action asks once more before it goes out.
                let mut ask = |ui: &mut egui::Ui, key: &'static str, label: &str, action: Moderation| {
                    let label = if *confirm == Some(key) { i18n::t("Confirm") } else { label };
                    if ui.button(RichText::new(label).color(rgb(t.error))).clicked() {
                        if *confirm == Some(key) {
                            *moderation = Some(action);
                            *confirm = None;
                        } else {
                            *confirm = Some(key);
                        }
                    }
                };
                if r.kick {
                    ask(ui, "kick", i18n::t("Kick"), Moderation::Kick);
                }
                if r.ban {
                    ask(ui, "ban", i18n::t("Ban"), Moderation::Ban);
                }
                if !r.recent.is_empty() {
                    ask(ui, "redact", &i18n::plural(r.recent.len() as u64, "Delete {n} message", "Delete {n} messages"), Moderation::RedactMessages(r.recent.clone()));
                }
                if failed {
                    ui.colored_label(rgb(t.error), i18n::t("Failed"));
                }
            }
        });
    });
    flip
}

/// What the thread drawer asks of the session this frame.
#[derive(Default)]
pub struct ThreadOut {
    pub send: Option<String>,
    pub voice: Option<crate::composer::VoiceAct>,
    pub picked: Option<std::path::PathBuf>,
    pub pasted: Option<egui::ColorImage>,
    pub save: Option<(matrix_domain::EventId, String)>,
}

/// Thread drawer: title bar, the root and its replies as the room's own bubbles (gathered at the bottom), and the room's composer bar.
/// The bars keep the panel's colour, the messages lie on the chat background. `recording` = seconds of the note being recorded here.
pub fn thread(ui: &mut egui::Ui, th: &mut crate::session::ThreadState, pictures: &crate::timeline::Pictures, avatars: &crate::timeline::Avatars, members: u64, recording: Option<u64>, acts: &mut crate::timeline::Acts, close: &mut bool, t: &DesignTokens) -> ThreadOut {
    let mut out = ThreadOut::default();
    egui::Panel::top("thread-head").show_separator_line(false).frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 10))).show_inside(ui, |ui| crate::dir::scope(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(8.);
            ui.label(RichText::new(i18n::t("Thread")).size(crate::metrics::get().title).color(rgb(t.text)));
            ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| {
                if close_icon(ui, i18n::t("Close thread")).clicked() {
                    *close = true;
                }
            });
        });
    }));
    egui::Panel::bottom("thread-input").show_separator_line(false).frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 4))).show_inside(ui, |ui| crate::dir::scope(ui, |ui| {
        if let Some((_, name)) = &th.reply_to {
            let label = if name.is_empty() { i18n::t_ctx("banner", "Reply").to_owned() } else { i18n::t("Reply to {name}").replace("{name}", name) };
            ui.horizontal(|ui| {
                ui.add_space(12.);
                ui.add(egui::Label::new(label).truncate());
                if crate::icons::button(ui, crate::icons::Icon::X, false, i18n::t("Cancel reply")).clicked() { th.reply_to = None; }
            });
        }
        let (mut cancel, mut drop, mut poll) = (false, false, false);
        let attachment = th.attachment.as_ref().map(|(n, data)| (n.as_str(), data.as_slice(), th.attachment_revision));
        out.send = crate::composer::show(ui, crate::composer::THREAD, &mut th.draft, crate::composer::Mode::Send, attachment, &mut cancel, &mut drop, &mut poll, &mut out.picked, &mut out.pasted, recording, &mut out.voice, &[], &mut Vec::new(), None, t);
        if drop { th.attachment = None; }
    }));
    th.tail_visible = false;
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(rgb(t.timeline))).show_inside(ui, |ui| crate::dir::scope(ui, |ui| {
        let Some(history) = &th.history else {
            ui.add_space(16.);
            if th.failed {
                ui.vertical_centered(|ui| ui.colored_label(rgb(t.error), i18n::t("Thread unavailable")));
            } else {
                ui.vertical_centered(|ui| ui.spinner());
            }
            return;
        };
        let top = ui.max_rect().center_top();
        let head = th.head();
        let (first, menu, save, _) = ui.push_id(("thread", th.root.as_str()), |ui| crate::timeline::show(ui, 0, history.events(), history, &mut th.heights, pictures, avatars, acts, members, &Default::default(), t, None, Some(head))).inner;
        out.save = save;
        let opened = menu.and_then(|(id, pos)| match id.id { matrix_domain::TimelineId::Server(id) => Some((id, pos)), _ => None });
        // One entry: «Reply». A click anywhere else (not the one that opened it) closes it.
        if let Some((id, pos)) = th.menu.clone().filter(|_| opened.is_none()) {
            let reply = egui::Area::new(egui::Id::new("thread-menu")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| egui::Frame::popup(ui.style()).fill(rgb(t.surface)).show(ui, |ui| ui.button(i18n::t("Reply")).clicked()).inner).inner;
            if reply {
                let name = history.row(&id).and_then(|e| e.sender.as_ref()).map(|s| s.name.clone().unwrap_or_else(|| s.id.clone())).unwrap_or_default();
                th.reply_to = Some((id, name));
            }
            if reply || ui.input(|i| i.pointer.any_click() || i.key_pressed(egui::Key::Escape)) { th.menu = None; }
        }
        if opened.is_some() { th.menu = opened; }
        th.tail_visible = th.heights.at_bottom;
        th.scrolled(first);
        if th.growing.is_some() {
            // Over the list, not in it: the rows do not move when it comes and goes.
            ui.put(egui::Rect::from_center_size(top + Vec2::new(0., 16.), Vec2::splat(16.)), egui::Spinner::new().size(16.));
        }
    }));
    out
}

/// The small copy button.
pub fn copy_icon(ui: &mut egui::Ui) -> egui::Response {
    crate::icons::button(ui, crate::icons::Icon::Copy, false, i18n::t("Copy"))
}

/// What the panel needs for the room's addresses and what the user did with them this frame.
pub struct AliasView<'a> {
    pub data: Option<&'a matrix_engine::alias::RoomAliases>,
    pub error: Option<&'static str>,
    pub input: &'a mut String,
    pub action: Option<matrix_engine::alias::AliasAction>,
}

fn alias_wrap(width: f32, server: f32, button: f32, gap: f32) -> bool {
    width < server + button * 3. + gap * 2.
}

fn addresses(ui: &mut egui::Ui, view: &mut AliasView, t: &DesignTokens) {
    use matrix_engine::alias::AliasAction;
    let Some(data) = view.data else { return };
    // Main first, then the other published ones, then the local ones that are not published.
    let mut list: Vec<String> = data.canonical.main.iter().cloned().collect();
    for a in data.canonical.alts.iter().chain(&data.local) {
        if !list.contains(a) {
            list.push(a.clone());
        }
    }
    if list.is_empty() && !data.can_edit_aliases {
        return;
    }
    ui.add_space(8.);
    ui.horizontal(|ui| {
        ui.add_space(16.);
        ui.with_layout(crate::dir::col(), |ui| {
            ui.set_max_width(ui.available_width() - 16.);
            ui.label(RichText::new(i18n::t("Addresses")).strong());
            for alias in &list {
                ui.horizontal_wrapped(|ui| {
                    ui.label(alias);
                    if data.canonical.main.as_deref() == Some(alias) {
                        ui.label(RichText::new(i18n::t("main")).size(11.).color(rgb(t.secondary)));
                    } else if data.can_edit_aliases && ui.small_button(i18n::t("Main")).clicked() {
                        view.action = Some(AliasAction::SetCanonical { alias: alias.clone() });
                    }
                    if data.can_edit_aliases && crate::info::close_icon(ui, i18n::t("Remove address")).clicked() {
                        view.action = Some(AliasAction::Remove { alias: alias.clone() });
                    }
                });
            }
            if data.can_edit_aliases {
                let server = format!(":{}", data.server);
                let font = egui::TextStyle::Body.resolve(ui.style());
                let server_width = ui.fonts_mut(|fonts| fonts.layout_no_wrap(server.clone(), font, rgb(t.secondary)).size().x);
                let available = ui.available_width();
                let gap = ui.spacing().item_spacing.x;
                let button = crate::metrics::button();
                let wrap = alias_wrap(available, server_width, button, gap);
                if wrap {
                    ui.add(crate::metrics::line(view.input).desired_width(available).hint_text(i18n::t("name")));
                }
                ui.horizontal(|ui| {
                    if wrap {
                        let indent = (available - server_width - button - gap * 2.).max(0.);
                        if indent > 0. { ui.add_space(indent); }
                    } else {
                        ui.add(crate::metrics::line(view.input).desired_width((available - server_width - button - gap * 2.).max(0.)).hint_text(i18n::t("name")));
                    }
                    ui.label(RichText::new(server).color(rgb(t.secondary)));
                    if crate::icons::button(ui, crate::icons::Icon::Plus, false, i18n::t("Add")).on_hover_text(i18n::t("Add")).clicked() && !view.input.trim().is_empty() {
                        view.action = Some(AliasAction::Add { local: view.input.trim().to_owned(), make_canonical: None });
                        view.input.clear();
                    }
                });
            }
            if let Some(e) = view.error {
                ui.colored_label(rgb(t.error), e);
            }
        });
    });
    ui.add_space(4.);
}

/// The small close button; `label` is its screen-reader name.
pub fn close_icon(ui: &mut egui::Ui, label: &str) -> egui::Response {
    crate::icons::button(ui, crate::icons::Icon::X, false, label)
}
