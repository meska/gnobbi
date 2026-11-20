//! «Nuovo sondaggio» window: question, options, the two switches.
use crate::session::Session;
use eframe::egui;
use matrix_desktop::i18n;
use matrix_domain::PollDraft;


pub struct Form {
    question: String,
    answers: Vec<String>,
    multiple: bool,
    disclosed: bool,
}

impl Default for Form {
    fn default() -> Self {
        Self { question: String::new(), answers: vec![String::new(); PollDraft::MIN_ANSWERS], multiple: false, disclosed: false }
    }
}

impl Form {
    fn add_option(&mut self) {
        if self.answers.len() < PollDraft::MAX_ANSWERS { self.answers.push(String::new()); }
    }
    fn remove_option(&mut self, i: usize) {
        if i >= PollDraft::MIN_ANSWERS && i < self.answers.len() { self.answers.remove(i); }
    }
}

pub fn show(ctx: &egui::Context, s: &mut Session, f: &mut Form, t: &matrix_desktop::DesignTokens) {
    let mut open = true;
    egui::Window::new(i18n::t("New poll")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(320.);
        ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut f.question).hint_text(i18n::t("Question")));
        let mut remove = None;
        egui::ScrollArea::vertical().max_height(300.).show(ui, |ui| {
            for (i, a) in f.answers.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 32.).max(0.);
                    ui.add_sized([width, crate::metrics::button()], crate::metrics::line(a).hint_text(i18n::t("Option {n}").replace("{n}", &(i + 1).to_string())));
                    if i >= PollDraft::MIN_ANSWERS && crate::icons::button(ui, crate::icons::Icon::X, false, i18n::t("Remove option")).clicked() { remove = Some(i); }
                });
            }
        });
        if let Some(i) = remove { f.remove_option(i); }
        if f.answers.len() < PollDraft::MAX_ANSWERS && ui.button(i18n::t("Add option")).clicked() { f.add_option(); }
        ui.add_space(4.);
        crate::settings::switch(ui, i18n::t("Multiple choice"), &mut f.multiple, t);
        crate::settings::switch(ui, i18n::t("Results visible right away"), &mut f.disclosed, t);
        ui.add_space(6.);
        let draft = PollDraft {
            question: f.question.trim().to_owned(),
            answers: f.answers.iter().map(|a| a.trim().to_owned()).filter(|a| !a.is_empty()).collect(),
            multiple: f.multiple,
            disclosed: f.disclosed,
        };
        let valid = draft.validate().is_ok();
        if ui.add_enabled(valid, egui::Button::new(i18n::t("Send"))).clicked() {
            s.send_poll(draft);
            *f = Form::default();
            s.poll_open = false;
        }
    });
    if !open {
        s.poll_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_respect_limits_and_remove_selected_row() {
        let mut f = Form::default();
        assert_eq!(f.answers.len(), PollDraft::MIN_ANSWERS);
        for _ in 0..PollDraft::MAX_ANSWERS + 2 { f.add_option(); }
        assert_eq!(f.answers.len(), PollDraft::MAX_ANSWERS);
        f.answers[2] = "terza".into();
        f.remove_option(2);
        assert!(!f.answers.iter().any(|a| a == "terza"));
        let before = f.answers.len();
        f.remove_option(0);
        f.remove_option(usize::MAX);
        assert_eq!(f.answers.len(), before);
    }
}
