//! Right panel: the open room's facts and its members.
use eframe::egui::{self, Align2, Color32, FontId, RichText, Vec2};
use matrix_desktop::{DesignTokens, SidebarRoom};
use matrix_engine::operations::Member;

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// What the user did in the panel this frame.
#[derive(Default)]
pub struct Out {
    pub member: Option<crate::members::Act>,
    pub edit: bool,
}

pub fn show(ui: &mut egui::Ui, room: &SidebarRoom, members: Option<&[Member]>, admin: Option<&crate::members::Admin>, me: &str, aliases: &mut AliasView, t: &DesignTokens) -> Out {
    let mut out = Out::default();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(16.);
        ui.horizontal(|ui| {
            ui.add_space(16.);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width() - 16.);
                ui.heading(&room.name);
                if let Some(topic) = room.info.topic.as_deref().filter(|t| !t.is_empty()) {
                    ui.add_space(4.);
                    ui.label(RichText::new(topic).color(rgb(t.secondary)));
                }
                if room.info.edit_rights.any() && ui.button("Modifica").clicked() {
                    out.edit = true;
                }
                ui.add_space(8.);
                ui.label(RichText::new(format!("{} membri", room.info.joined_members)).color(rgb(t.secondary)));
                if room.info.encrypted == Some(true) {
                    ui.label(RichText::new("Cifrata").color(rgb(t.secondary)));
                }
            });
        });
        ui.add_space(12.);
        ui.separator();
        addresses(ui, aliases, t);
        let Some(members) = members else {
            ui.add_space(12.);
            ui.vertical_centered(|ui| ui.spinner());
            return;
        };
        out.member = crate::members::list(ui, members, admin, me, t);
    });
    out
}

/// One member: initials and name on a full-width row.
pub fn member_row(ui: &mut egui::Ui, m: &Member, t: &DesignTokens) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0., rgb(t.input));
    }
    let c = egui::pos2(rect.left() + 28., rect.center().y);
    let initials: String = m.name.trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
    ui.painter().circle_filled(c, 14., rgb(matrix_desktop::AVATAR_FALLBACK));
    ui.painter().text(c, Align2::CENTER_CENTER, initials, FontId::proportional(12.), rgb(t.on_accent));
    let galley = ui.painter().layout(m.name.clone(), FontId::proportional(14.), rgb(t.text), (rect.width() - 60.).max(20.));
    ui.painter().galley(egui::pos2(rect.left() + 52., rect.center().y - galley.size().y / 2.), galley, rgb(t.text));
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
        ui.add_sized([w, 32.], egui::TextEdit::singleline(term).hint_text("Cerca nella stanza").align(egui::Align2::LEFT_CENTER));
    });
    ui.add_space(8.);
    match state {
        S::Idle => {}
        S::Searching { .. } => {
            ui.vertical_centered(|ui| ui.spinner());
        }
        S::NoResults { .. } => {
            ui.label(RichText::new("Nessun risultato").color(rgb(t.secondary)));
        }
        S::Error { .. } => {
            ui.colored_label(rgb(t.error), "Ricerca non riuscita");
        }
        S::Results { hits, more, .. } => {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for h in hits {
                    ui.add_space(6.);
                    let response = ui.horizontal(|ui| {
                        ui.add_space(12.);
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width() - 12.);
                            ui.label(RichText::new(&h.sender_name).strong().size(12.).color(rgb(t.accent)));
                            ui.label(&h.excerpt);
                        });
                    }).response.interact(egui::Sense::click());
                    if response.clicked() {
                        selected = Some((matrix_domain::EventId::new(h.thread_root.as_deref().unwrap_or(&h.event_id)), h.thread_root.is_some()));
                    }
                    ui.separator();
                }
                match more {
                    matrix_engine::operations::MoreHits::Available | matrix_engine::operations::MoreHits::Failed => {
                        let label = if matches!(more, matrix_engine::operations::MoreHits::Failed) { "Riprova" } else { "Altri risultati" };
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
pub fn card(ui: &mut egui::Ui, id: &str, profile: Option<&matrix_engine::operations::UserProfile>, rights: Option<&matrix_engine::operations::MemberRights>, failed: bool, dm_failed: bool, me: bool, can_verify: bool, action: &mut Option<ProfileAction>, confirm: &mut Option<&'static str>, moderation: &mut Option<matrix_engine::operations::Moderation>, close: &mut bool, t: &DesignTokens) -> Option<bool> {
    let mut flip = None;
    ui.add_space(16.);
    ui.horizontal(|ui| {
        ui.add_space(16.);
        if crate::info::close_icon(ui).clicked() {
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
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() - 16.);
            ui.heading(&p.name);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(id).color(rgb(t.secondary)));
                if copy_icon(ui).clicked() {
                    ui.ctx().copy_text(id.to_owned());
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Link profilo");
                if copy_icon(ui).clicked() {
                    ui.ctx().copy_text(format!("https://matrix.to/#/{id}"));
                }
            });
            match p.verified {
                Some(true) => ui.label("Verificato"),
                Some(false) => ui.label(RichText::new("Non verificato").color(rgb(t.error))),
                None => ui.label(""),
            };
            if !me {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Messaggio").clicked() { *action = Some(ProfileAction::Message); }
                    if ui.button("Menziona").clicked() { *action = Some(ProfileAction::Mention); }
                    if p.verified == Some(false) && ui.add_enabled(can_verify, egui::Button::new("Verifica")).clicked() { *action = Some(ProfileAction::Verify); }
                    if rights.is_some_and(|r| r.read_receipt.is_some()) && ui.button("Ultima lettura").clicked() { *action = Some(ProfileAction::ReadReceipt); }
                });
                if dm_failed { ui.colored_label(rgb(t.error), "Chat non disponibile"); }
            }
            ui.add_space(8.);
            let mut ignored = p.ignored;
            if crate::settings::switch(ui, "Ignora", &mut ignored, t) {
                flip = Some(ignored);
            }
            if let Some(r) = rights {
                use matrix_engine::operations::Moderation;
                ui.add_space(8.);
                ui.separator();
                // Each destructive action asks once more before it goes out.
                let mut ask = |ui: &mut egui::Ui, key: &'static str, label: &str, action: Moderation| {
                    let label = if *confirm == Some(key) { "Conferma" } else { label };
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
                    ask(ui, "kick", "Espelli", Moderation::Kick);
                }
                if r.ban {
                    ask(ui, "ban", "Blocca", Moderation::Ban);
                }
                if !r.recent.is_empty() {
                    ask(ui, "redact", &format!("Elimina {} messaggi", r.recent.len()), Moderation::RedactMessages(r.recent.clone()));
                }
                for level in &r.levels {
                    let name = match level {
                        100 => "Amministratore",
                        50 => "Moderatore",
                        _ => "Utente",
                    };
                    if ui.button(format!("Ruolo: {name}")).clicked() {
                        *moderation = Some(Moderation::SetPower(*level));
                    }
                }
                if failed {
                    ui.colored_label(rgb(t.error), "Non riuscito");
                }
            }
        });
    });
    flip
}

/// Thread panel: the root, its replies (oldest first) and a line to answer in the thread. Returns the text to send.
pub fn thread(ui: &mut egui::Ui, th: &mut crate::session::ThreadState, close: &mut bool, t: &DesignTokens) -> Option<String> {
    let mut send = None;
    ui.add_space(12.);
    ui.horizontal(|ui| {
        ui.add_space(12.);
        if crate::info::close_icon(ui).clicked() {
            *close = true;
        }
    });
    egui::Panel::bottom("thread-input").show_separator_line(false).frame(egui::Frame::NONE).show_inside(ui, |ui| {
        ui.add_space(6.);
        if let Some((_, name)) = &th.reply_to {
            let name = name.clone();
            ui.horizontal(|ui| {
                ui.label(format!("Risposta a {name}"));
                if ui.button("×").clicked() { th.reply_to = None; }
            });
        }
        let (mut cancel, mut drop, mut poll, mut picked, mut pasted, mut voice) = (false, false, false, None, None, None);
        send = crate::composer::show(ui, crate::composer::THREAD, &mut th.draft, crate::composer::Mode::Send, None, &mut cancel, &mut drop, &mut poll, &mut picked, &mut pasted, None, &mut voice, &[], &mut Vec::new());
        ui.add_space(6.);
    });
    let Some(page) = &th.page else {
        ui.add_space(16.);
        if th.failed {
            ui.colored_label(rgb(t.error), "Thread non disponibile");
        } else {
            ui.vertical_centered(|ui| ui.spinner());
        }
        return send;
    };
    th.tail_visible = false;
    egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
        if !page.at_start && !th.stalled && th.growing.is_none() && ui.button("Carica precedenti").clicked() {
            th.growing = Some(page.replies.len());
            th.want = th.want.saturating_add(50);
            th.next_read = std::time::Instant::now();
        }
        for e in page.root.iter().chain(page.replies.iter()) {
            ui.add_space(6.);
            let row = ui.horizontal(|ui| {
                ui.add_space(12.);
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - 12.);
                    let name = e.sender.name.clone().unwrap_or_else(|| e.sender.id.clone());
                    ui.label(RichText::new(name).strong().size(12.).color(rgb(t.accent)));
                    if e.redacted {
                        ui.label(RichText::new("Messaggio eliminato").italics().color(rgb(t.secondary)));
                    } else {
                        ui.label(&e.body);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        ui.label(RichText::new(matrix_desktop::bubble_stamp(&matrix_desktop::time_fmt::clock_time(e.timestamp_ms), e.outgoing, None)).size(10.).color(rgb(t.secondary)));
                    });
                });
            }).response.interact(egui::Sense::click());
            row.context_menu(|ui| {
                if ui.button("Rispondi").clicked() {
                    th.reply_to = Some((e.event_id.clone(), e.sender.name.clone().unwrap_or_else(|| e.sender.id.clone())));
                    ui.close();
                }
            });
            if page.replies.last().or(page.root.as_ref()).is_some_and(|last| last.event_id == e.event_id) {
                th.tail_visible = ui.clip_rect().contains_rect(row.rect);
            }
            ui.separator();
        }
    });
    send
}

/// The small copy icon (two sheets), drawn: the font has no glyph for it everywhere.
pub fn copy_icon(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.), egui::Sense::click());
    let color = if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let stroke = egui::Stroke::new(1.3, color);
    let c = rect.center();
    ui.painter().rect_stroke(egui::Rect::from_min_size(c + Vec2::new(-6., -7.), Vec2::new(9., 11.)), 1.5, stroke, egui::StrokeKind::Inside);
    ui.painter().rect_stroke(egui::Rect::from_min_size(c + Vec2::new(-3., -4.), Vec2::new(9., 11.)), 1.5, stroke, egui::StrokeKind::Inside);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What the panel needs for the room's addresses and what the user did with them this frame.
pub struct AliasView<'a> {
    pub data: Option<&'a matrix_engine::alias::RoomAliases>,
    pub error: Option<&'static str>,
    pub input: &'a mut String,
    pub action: Option<matrix_engine::alias::AliasAction>,
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
        ui.vertical(|ui| {
            ui.label(RichText::new("Indirizzi").strong());
            for alias in &list {
                ui.horizontal_wrapped(|ui| {
                    ui.label(alias);
                    if data.canonical.main.as_deref() == Some(alias) {
                        ui.label(RichText::new("principale").size(11.).color(rgb(t.secondary)));
                    } else if data.can_edit_aliases && ui.small_button("Principale").clicked() {
                        view.action = Some(AliasAction::SetCanonical { alias: alias.clone() });
                    }
                    if data.can_edit_aliases && crate::info::close_icon(ui).clicked() {
                        view.action = Some(AliasAction::Remove { alias: alias.clone() });
                    }
                });
            }
            if data.can_edit_aliases {
                // Wrapped: field + server + button are wider than the 300 px panel and made its content overflow on both sides.
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(view.input).desired_width(110.).hint_text("nome").align(egui::Align2::LEFT_CENTER));
                    ui.label(RichText::new(format!(":{}", data.server)).color(rgb(t.secondary)));
                    if ui.button("Aggiungi").clicked() && !view.input.trim().is_empty() {
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
    ui.separator();
}

/// The small X, drawn: the glyph is missing from the macOS system font.
pub fn close_icon(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.), egui::Sense::click());
    let color = if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let c = rect.center();
    let stroke = egui::Stroke::new(1.4, color);
    ui.painter().line_segment([c + Vec2::new(-4., -4.), c + Vec2::new(4., 4.)], stroke);
    ui.painter().line_segment([c + Vec2::new(-4., 4.), c + Vec2::new(4., -4.)], stroke);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
