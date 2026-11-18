//! «Nuovo sondaggio» window: question, up to four options, the two switches.
use crate::session::Session;
use eframe::egui;
use matrix_domain::PollDraft;

#[derive(Default)]
pub struct Form {
    question: String,
    answers: [String; 4],
    multiple: bool,
    disclosed: bool,
}

pub fn show(ctx: &egui::Context, s: &mut Session, f: &mut Form, t: &matrix_desktop::DesignTokens) {
    let mut open = true;
    egui::Window::new("Nuovo sondaggio").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(320.);
        ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(&mut f.question).hint_text("Domanda"));
        for (i, a) in f.answers.iter_mut().enumerate() {
            ui.add_sized([ui.available_width(), 28.], egui::TextEdit::singleline(a).hint_text(format!("Opzione {}", i + 1)));
        }
        ui.add_space(4.);
        crate::settings::switch(ui, "Scelta multipla", &mut f.multiple, t);
        crate::settings::switch(ui, "Risultati visibili subito", &mut f.disclosed, t);
        ui.add_space(6.);
        let draft = PollDraft {
            question: f.question.trim().to_owned(),
            answers: f.answers.iter().map(|a| a.trim().to_owned()).filter(|a| !a.is_empty()).collect(),
            multiple: f.multiple,
            disclosed: f.disclosed,
        };
        let valid = draft.validate().is_ok();
        if ui.add_enabled(valid, egui::Button::new("Invia")).clicked() {
            s.send_poll(draft);
            *f = Form::default();
            s.poll_open = false;
        }
    });
    if !open {
        s.poll_open = false;
    }
}
