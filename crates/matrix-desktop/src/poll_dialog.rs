//! «Crea sondaggio»: modal dialog opened from the composer. Same container as `new_room_view.rs`.
//! Only field labels and one-line errors: no help text.
use crate::login_field::LoginField;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, Window, div, prelude::*, px, rgb,
};
use matrix_desktop::DesignTokens;
use matrix_domain::{PollDraft, PollDraftError};

pub(crate) enum PollDialogEvent {
    Create(PollDraft),
    Cancel,
}

const OPTION_LABELS: [&str; PollDraft::MAX_ANSWERS] = [
    "Opzione 1", "Opzione 2", "Opzione 3", "Opzione 4", "Opzione 5", "Opzione 6", "Opzione 7",
    "Opzione 8", "Opzione 9", "Opzione 10", "Opzione 11", "Opzione 12", "Opzione 13", "Opzione 14",
    "Opzione 15", "Opzione 16", "Opzione 17", "Opzione 18", "Opzione 19", "Opzione 20",
];
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
const REMOVE_LABELS: [&str; PollDraft::MAX_ANSWERS] = [
    "Rimuovi opzione 1", "Rimuovi opzione 2", "Rimuovi opzione 3", "Rimuovi opzione 4",
    "Rimuovi opzione 5", "Rimuovi opzione 6", "Rimuovi opzione 7", "Rimuovi opzione 8",
    "Rimuovi opzione 9", "Rimuovi opzione 10", "Rimuovi opzione 11", "Rimuovi opzione 12",
    "Rimuovi opzione 13", "Rimuovi opzione 14", "Rimuovi opzione 15", "Rimuovi opzione 16",
    "Rimuovi opzione 17", "Rimuovi opzione 18", "Rimuovi opzione 19", "Rimuovi opzione 20",
];

/// A tab stop: the question (field 0), option `i` (field 1 + i), its remove icon, the buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    Question,
    Option(usize),
    Remove(usize),
    Add,
    Multiple,
    Disclosed,
    Cancel,
    Create,
}

pub(crate) struct PollDialog {
    pub(crate) theme: DesignTokens,
    /// Field 0 is the question, then one per option.
    fields: Vec<Entity<LoginField>>,
    /// Remove icons, parallel to the options (the first two have none).
    removes: Vec<FocusHandle>,
    add: FocusHandle,
    multiple_focus: FocusHandle,
    disclosed_focus: FocusHandle,
    cancel: FocusHandle,
    create: FocusHandle,
    multiple: bool,
    disclosed: bool,
    /// Fields that lost focus once; only those show their error.
    touched: Vec<bool>,
    last_field: Option<usize>,
}

impl EventEmitter<PollDialogEvent> for PollDialog {}

/// The one-line error of the first invalid field, as (field index, text).
fn error_of(draft: &PollDraft) -> Option<(usize, &'static str)> {
    use PollDraftError::*;
    Some(match draft.validate().err()? {
        EmptyQuestion => (0, "Domanda mancante"),
        QuestionTooLong => (0, "Massimo 340 caratteri"),
        EmptyAnswer(i) => (1 + i, "Opzione mancante"),
        AnswerTooLong(i) => (1 + i, "Massimo 340 caratteri"),
        DuplicateAnswer(i) => (1 + i, "Opzione ripetuta"),
        TooFewAnswers | TooManyAnswers => return None,
    })
}

impl PollDialog {
    pub(crate) fn new(theme: DesignTokens, cx: &mut Context<Self>) -> Self {
        let mut dialog = Self {
            theme,
            fields: Vec::new(),
            removes: Vec::new(),
            add: cx.focus_handle(),
            multiple_focus: cx.focus_handle(),
            disclosed_focus: cx.focus_handle(),
            cancel: cx.focus_handle(),
            create: cx.focus_handle(),
            multiple: false,
            disclosed: true,
            touched: Vec::new(),
            last_field: None,
        };
        dialog.push_field("Domanda", cx);
        for _ in 0..PollDraft::MIN_ANSWERS {
            dialog.add_option(cx);
        }
        dialog
    }

    fn push_field(&mut self, label: &'static str, cx: &mut Context<Self>) {
        let field = cx.new(|cx| {
            let mut field = LoginField::new(false, cx);
            field.enabled = true;
            field.placeholder = label;
            field
        });
        cx.observe(&field, |_, _, cx| cx.notify()).detach();
        self.fields.push(field);
        self.touched.push(false);
    }

    fn options(&self) -> usize {
        self.fields.len() - 1
    }

    fn add_option(&mut self, cx: &mut Context<Self>) {
        if self.options() < PollDraft::MAX_ANSWERS {
            self.push_field(OPTION_LABELS[self.options()], cx);
            self.removes.push(cx.focus_handle());
        }
    }

    pub(crate) fn focus_question(&self, window: &mut Window, cx: &gpui::App) {
        self.fields[0].read(cx).focus.focus(window);
    }

    pub(crate) fn draft(&self, cx: &gpui::App) -> PollDraft {
        let text = |f: &Entity<LoginField>| f.read(cx).input.display_text().trim().to_owned();
        PollDraft {
            question: text(&self.fields[0]),
            answers: self.fields[1..].iter().map(text).collect(),
            multiple: self.multiple,
            disclosed: self.disclosed,
        }
    }

    fn valid(&self, cx: &gpui::App) -> bool {
        self.draft(cx).validate().is_ok()
    }

    /// Tab order; «Crea» is a stop only while it is enabled.
    pub(crate) fn stops(&self, cx: &gpui::App) -> Vec<(Stop, FocusHandle)> {
        let mut stops = vec![(Stop::Question, self.fields[0].read(cx).focus.clone())];
        for i in 0..self.options() {
            stops.push((Stop::Option(i), self.fields[1 + i].read(cx).focus.clone()));
            if i >= PollDraft::MIN_ANSWERS {
                stops.push((Stop::Remove(i), self.removes[i].clone()));
            }
        }
        if self.options() < PollDraft::MAX_ANSWERS {
            stops.push((Stop::Add, self.add.clone()));
        }
        stops.push((Stop::Multiple, self.multiple_focus.clone()));
        stops.push((Stop::Disclosed, self.disclosed_focus.clone()));
        stops.push((Stop::Cancel, self.cancel.clone()));
        if self.valid(cx) {
            stops.push((Stop::Create, self.create.clone()));
        }
        stops
    }

    #[cfg(test)]
    pub(crate) fn focused(&self, window: &Window, cx: &gpui::App) -> Option<Stop> {
        self.stops(cx).into_iter().find(|(_, h)| h.is_focused(window)).map(|(s, _)| s)
    }

    fn remove_option(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if i < PollDraft::MIN_ANSWERS || i >= self.options() {
            return;
        }
        self.fields.remove(1 + i);
        self.touched.remove(1 + i);
        self.removes.remove(i);
        self.last_field = None;
        self.fields[1 + i.min(self.options() - 1)].read(cx).focus.focus(window);
        cx.notify();
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        if self.valid(cx) {
            cx.emit(PollDialogEvent::Create(self.draft(cx)));
        }
    }

    fn activate(&mut self, stop: Stop, window: &mut Window, cx: &mut Context<Self>) {
        match stop {
            Stop::Remove(i) => self.remove_option(i, window, cx),
            Stop::Add => {
                self.add_option(cx);
                self.fields[self.options()].read(cx).focus.focus(window);
            }
            Stop::Multiple => self.multiple = !self.multiple,
            Stop::Disclosed => self.disclosed = !self.disclosed,
            Stop::Cancel => cx.emit(PollDialogEvent::Cancel),
            Stop::Create => self.create(cx),
            Stop::Question | Stop::Option(_) => {}
        }
        cx.notify();
    }

    pub(crate) fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let stops = self.stops(cx);
        let at = stops.iter().position(|(_, h)| h.is_focused(window));
        if key == "tab" {
            let n = stops.len();
            let next = match (at, event.keystroke.modifiers.shift) {
                (Some(at), true) => (at + n - 1) % n,
                (Some(at), false) => (at + 1) % n,
                (None, _) => 0,
            };
            stops[next].1.focus(window);
        } else if key == "escape" {
            cx.emit(PollDialogEvent::Cancel);
        } else if let Some(at) = at
            && matches!(stops[at].0, Stop::Question | Stop::Option(_))
            && key == "enter"
        {
            self.create(cx);
        } else if let Some(at) = at
            && !matches!(stops[at].0, Stop::Question | Stop::Option(_))
            && matches!(key, "enter" | "space")
        {
            self.activate(stops[at].0, window, cx);
        } else {
            return;
        }
        cx.notify();
        cx.stop_propagation();
    }

    /// Text-input labels and buttons in Tab order, for the screen-reader tree.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic(&self, window: &Window, cx: &gpui::App) -> matrix_desktop::accessibility::new_room::Snapshot {
        use accesskit::Role;
        use matrix_desktop::accessibility::new_room::{Item, Snapshot};
        let mut items: Vec<Item> = self
            .stops(cx)
            .into_iter()
            .map(|(stop, focus)| {
                let (role, label, checked) = match stop {
                    Stop::Question => (Role::TextInput, "Domanda", None),
                    Stop::Option(i) => (Role::TextInput, OPTION_LABELS[i], None),
                    Stop::Remove(i) => (Role::Button, REMOVE_LABELS[i], None),
                    Stop::Add => (Role::Button, "Aggiungi opzione", None),
                    Stop::Multiple => (Role::CheckBox, "Scelta multipla", Some(self.multiple)),
                    Stop::Disclosed => (Role::CheckBox, "Risultati visibili", Some(self.disclosed)),
                    Stop::Cancel => (Role::Button, "Annulla", None),
                    Stop::Create => (Role::Button, "Crea", None),
                };
                Item { role, label: label.to_owned(), checked, focused: focus.is_focused(window), bounds: None }
            })
            .collect();
        if let Some((field, text)) = error_of(&self.draft(cx)).filter(|(f, _)| self.touched[*f]) {
            let _ = field;
            items.push(Item { role: Role::Label, label: text.to_owned(), checked: None, focused: false, bounds: None });
        }
        Snapshot { items }
    }
}

impl Render for PollDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        // A field that held focus on the previous frame and no longer does has been left: its error may show.
        let now = self.fields.iter().position(|f| f.read(cx).focus.is_focused(window));
        if let Some(prev) = self.last_field.filter(|p| Some(*p) != now) {
            self.touched[prev] = true;
        }
        self.last_field = now;
        let error = error_of(&self.draft(cx)).filter(|(f, _)| self.touched[*f]);
        let valid = self.valid(cx);
        let ring = |focus: &FocusHandle| {
            rgb(if focus.is_focused(window) { t.accent } else { t.surface })
        };
        let line = |text: &'static str| div().text_size(px(13.)).text_color(rgb(t.error)).child(text);
        let button = |id: &'static str, label: &'static str, color: u32, focus: &FocusHandle| {
            div().id(id).track_focus(focus).px_4().py_2().rounded_md().cursor_pointer().border_1().border_color(ring(focus)).text_color(rgb(color)).child(label)
        };
        let switch = |id: &'static str, label: &'static str, on: bool, focus: &FocusHandle, cx: &mut Context<Self>| {
            div()
                .id(id)
                .track_focus(focus)
                .h(px(36.))
                .px_2()
                .flex()
                .items_center()
                .justify_between()
                .cursor_pointer()
                .border_1()
                .border_color(ring(focus))
                .text_color(rgb(t.text))
                .child(label)
                .child(
                    div()
                        .w(px(36.))
                        .h(px(20.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .px(px(2.))
                        .bg(rgb(if on { t.accent } else { t.disabled }))
                        .when(on, |d| d.justify_end())
                        .child(div().size(px(16.)).rounded_full().bg(rgb(t.on_accent))),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    let stop = if id == "poll-multiple" { Stop::Multiple } else { Stop::Disclosed };
                    this.activate(stop, window, cx);
                    let focus = if id == "poll-multiple" { &this.multiple_focus } else { &this.disclosed_focus };
                    focus.focus(window);
                }))
        };
        for field in &self.fields {
            field.update(cx, |f, _| f.theme = t);
        }
        let rows = self.fields.iter().enumerate().map(|(n, field)| {
            let remove = (n > PollDraft::MIN_ANSWERS).then(|| {
                let i = n - 1;
                let focus = &self.removes[i];
                div()
                    .id(("poll-remove", i))
                    .track_focus(focus)
                    .size(px(32.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_lg()
                    .cursor_pointer()
                    .border_1()
                    .border_color(ring(focus))
                    .child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.accent))
                    .on_click(cx.listener(move |this, _, window, cx| this.remove_option(i, window, cx)))
            });
            div()
                .flex()
                .flex_col()
                .child(div().flex().items_center().gap_2().child(div().flex_1().child(field.clone())).children(remove))
                .children(error.filter(|(f, _)| *f == n).map(|(_, text)| line(text)))
                }).collect::<Vec<_>>();
        div()
            .id("poll-layer")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(matrix_desktop::SCRIM))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(PollDialogEvent::Cancel)))
            .child(
                div()
                    .id("poll-dialog")
                    .w(px(360.))
                    .p_4()
                    .gap_2()
                    .flex()
                    .flex_col()
                    .rounded_lg()
                    .bg(rgb(t.surface))
                    .border_1()
                    .border_color(rgb(t.border))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.key(event, window, cx)))
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(t.text)).child("Nuovo sondaggio"))
                    .child(div().id("poll-fields").flex().flex_col().gap_2().max_h(px(360.)).overflow_y_scroll().children(rows))
                    .when(self.options() < PollDraft::MAX_ANSWERS, |d| {
                        d.child(button("poll-add", "Aggiungi opzione", t.accent, &self.add).on_click(cx.listener(|this, _, window, cx| {
                            this.activate(Stop::Add, window, cx)
                        })))
                    })
                    .child(switch("poll-multiple", "Scelta multipla", self.multiple, &self.multiple_focus, cx))
                    .child(switch("poll-disclosed", "Risultati visibili", self.disclosed, &self.disclosed_focus, cx))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(button("poll-cancel", "Annulla", t.accent, &self.cancel).on_click(cx.listener(|_, _, _, cx| cx.emit(PollDialogEvent::Cancel))))
                            .child(
                                button("poll-create", "Crea", if valid { t.accent } else { t.disabled }, &self.create)
                                    .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(question: &str, answers: &[&str]) -> PollDraft {
        PollDraft { question: question.into(), answers: answers.iter().map(|a| (*a).into()).collect(), ..Default::default() }
    }

    #[test]
    fn errors_name_the_field_and_stay_one_line() {
        assert_eq!(error_of(&draft("", &["a", "b"])), Some((0, "Domanda mancante")));
        assert_eq!(error_of(&draft("q", &["a", ""])), Some((2, "Opzione mancante")));
        assert_eq!(error_of(&draft("q", &["a", "b", "A"])), Some((3, "Opzione ripetuta")));
        assert_eq!(error_of(&draft("q", &["a", &"x".repeat(341)])), Some((2, "Massimo 340 caratteri")));
        assert_eq!(error_of(&draft("q", &["a", "b"])), None);
    }

    use std::{cell::RefCell, rc::Rc};

    type Seen = Rc<RefCell<Vec<Option<PollDraft>>>>;
    type Step = Box<dyn FnOnce(&mut PollDialog, &mut Window, &mut Context<PollDialog>, &Seen)>;

    fn key(k: &str) -> KeyDownEvent {
        KeyDownEvent { keystroke: gpui::Keystroke::parse(k).unwrap(), is_held: false }
    }

    fn type_into(d: &PollDialog, field: usize, text: &str, cx: &mut Context<PollDialog>) {
        d.fields[field].update(cx, |f, _| {
            f.input.replace(None, text);
        });
    }

    /// One dialog in a real window; each step runs after a repaint, so blur tracking sees the frames.
    /// `seen` collects Create(draft) as Some and Cancel as None.
    fn run(steps: Vec<Step>) {
        assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
        gpui::Application::new().run(move |cx| {
            let bounds = gpui::Bounds::centered(None, gpui::size(px(800.), px(700.)), cx);
            let seen: Seen = Rc::default();
            let view = cx
                .open_window(gpui::WindowOptions { window_bounds: Some(gpui::WindowBounds::Windowed(bounds)), ..Default::default() }, |window, cx| {
                    let dialog = cx.new(|cx| PollDialog::new(matrix_desktop::LIGHT, cx));
                    let log = seen.clone();
                    cx.subscribe(&dialog, move |_, event: &PollDialogEvent, _| {
                        log.borrow_mut().push(match event {
                            PollDialogEvent::Create(draft) => Some(draft.clone()),
                            PollDialogEvent::Cancel => None,
                        })
                    })
                    .detach();
                    dialog.read(cx).focus_question(window, cx);
                    dialog
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                for step in steps {
                    executor.timer(std::time::Duration::from_millis(250)).await;
                    view.update(cx, |d, window, cx| step(d, window, cx, &seen)).unwrap();
                }
                executor.timer(std::time::Duration::from_millis(250)).await;
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn tab_order_and_initial_focus() {
        run(vec![Box::new(|d, w, cx, _| {
            assert_eq!(d.focused(w, cx), Some(Stop::Question), "focus starts on Domanda");
            type_into(d, 0, "Pranzo?", cx);
            type_into(d, 1, "Pizza", cx);
            type_into(d, 2, "Sushi", cx);
            d.activate(Stop::Add, w, cx);
            assert_eq!(d.focused(w, cx), Some(Stop::Option(2)), "the new option takes focus");
            type_into(d, 3, "Gelato", cx);
            let mut order = vec![];
            for _ in 0..8 {
                d.key(&key("tab"), w, cx);
                order.push(d.focused(w, cx).unwrap());
            }
            use Stop::*;
            assert_eq!(order, [Remove(2), Add, Multiple, Disclosed, Cancel, Create, Question, Option(0)]);
            d.key(&key("shift-tab"), w, cx);
            assert_eq!(d.focused(w, cx), Some(Question));
            d.key(&key("shift-tab"), w, cx);
            assert_eq!(d.focused(w, cx), Some(Create), "wraps inside the dialog");
        })]);
    }

    #[test]
    #[ignore = "requires private display"]
    fn escape_cancels_and_cancel_button_too() {
        run(vec![
            Box::new(|d, w, cx, seen| {
                d.key(&key("escape"), w, cx);
            }),
            Box::new(|d, w, cx, seen| {
                assert_eq!(*seen.borrow(), [None], "Esc cancels");
                d.stops(cx).into_iter().find(|(s, _)| *s == Stop::Cancel).unwrap().1.focus(w);
                d.key(&key("enter"), w, cx);
            }),
            Box::new(|_, _, _, seen| assert_eq!(*seen.borrow(), [None, None], "Annulla cancels")),
        ]);
    }

    #[test]
    #[ignore = "requires private display"]
    fn create_is_disabled_until_the_draft_is_valid_and_defaults_are_set() {
        run(vec![
            Box::new(|d, w, cx, seen| {
                assert!(!d.valid(cx));
                assert!(d.stops(cx).iter().all(|(s, _)| *s != Stop::Create), "Crea is not a stop while invalid");
                d.create(cx);
                d.key(&key("enter"), w, cx);
                assert!(seen.borrow().is_empty(), "no draft leaves an invalid dialog");
                assert!(!d.multiple && d.disclosed, "Scelta multipla spenta, Risultati visibili acceso");
                type_into(d, 0, "  Pranzo?  ", cx);
                type_into(d, 1, "Pizza", cx);
                type_into(d, 2, "Sushi", cx);
            }),
            Box::new(|d, w, cx, seen| {
                assert!(d.valid(cx));
                d.activate(Stop::Multiple, w, cx);
                d.stops(cx).into_iter().find(|(s, _)| *s == Stop::Create).unwrap().1.focus(w);
                d.key(&key("enter"), w, cx);
            }),
            Box::new(|_, _, _, seen| {
                assert_eq!(
                    *seen.borrow(),
                    [Some(PollDraft { question: "Pranzo?".into(), answers: vec!["Pizza".into(), "Sushi".into()], multiple: true, disclosed: true })]
                );
            }),
        ]);
    }

    #[test]
    #[ignore = "requires private display"]
    fn duplicate_option_error_shows_only_after_blur_under_its_field() {
        run(vec![
            Box::new(|d, w, cx, _| {
                type_into(d, 0, "Pranzo?", cx);
                type_into(d, 1, "Pizza", cx);
                d.fields[2].read(cx).focus.focus(w);
                type_into(d, 2, "pizza", cx);
            }),
            Box::new(|d, w, cx, _| {
                assert_eq!(error_of(&d.draft(cx)), Some((2, "Opzione ripetuta")));
                assert!(!d.touched[2], "still typing in it: no error yet");
                d.key(&key("tab"), w, cx);
            }),
            Box::new(|d, w, cx, _| {
                assert!(d.touched[2], "left the field: the error shows");
                assert!(!d.valid(cx));
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                assert!(d.semantic(w, cx).items.iter().any(|i| i.label == "Opzione ripetuta"));
                let _ = w;
            }),
        ]);
    }

    #[test]
    #[ignore = "requires private display"]
    fn options_add_up_to_20_and_remove_from_the_third() {
        run(vec![Box::new(|d, w, cx, _| {
            for _ in 0..30 {
                d.activate(Stop::Add, w, cx);
            }
            assert_eq!(d.options(), PollDraft::MAX_ANSWERS);
            assert!(d.stops(cx).iter().all(|(s, _)| *s != Stop::Add), "Aggiungi opzione disappears at 20");
            assert!(d.stops(cx).iter().all(|(s, _)| *s != Stop::Remove(0) && *s != Stop::Remove(1)));
            type_into(d, 3, "terza", cx);
            d.remove_option(0, w, cx);
            assert_eq!(d.options(), 20, "the first two cannot be removed");
            d.remove_option(2, w, cx);
            assert_eq!(d.options(), 19);
            assert_eq!(d.draft(cx).answers[2], "");
            assert_eq!(d.focused(w, cx), Some(Stop::Option(2)));
        })]);
    }
}
