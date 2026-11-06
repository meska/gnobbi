//! Poll bubble (read-only): question, options with a marker, votes and bars when the results are
//! visible, footer. Colours are the existing tokens. Meaning for screen readers lives in the a11y tree.
use crate::composer_icons::{Icon, icon};
use gpui::{AnyElement, App, Window, Bounds, FontWeight, Pixels, canvas, div, prelude::*, px, relative, rgb};
use matrix_desktop::DesignTokens;
use matrix_domain::Poll;
use std::rc::Rc;

/// Parts whose painted bounds are recorded (a11y bounds, tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PollPart {
    Bubble,
    Option(usize),
    Votes(usize),
    Bar(usize),
}

pub(crate) type PollPainted = Rc<dyn Fn(PollPart, Bounds<Pixels>)>;

const MARKER_PX: f32 = 18.;

pub(crate) type PollVote = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// `row_ix` keeps element ids unique across timeline rows; `focused` is the keyboard option; `failed` shows the vote error.
pub(crate) fn poll_bubble(poll: &Poll, t: DesignTokens, row_ix: usize, focused: Option<usize>, failed: bool, on_vote: PollVote, painted: PollPainted) -> AnyElement {
    let probe = {
        let painted = painted.clone();
        move |part: PollPart| {
            let painted = painted.clone();
            canvas(|_, _, _| (), move |bounds, _, _, _| painted(part, bounds)).absolute().inset_0()
        }
    };
    let shown = poll.results_visible();
    let options = (0..poll.answers.len()).map(|ix| {
        let a = &poll.answers[ix];
        let marker = div()
            .size(px(MARKER_PX))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(rgb(if a.own { t.accent } else { t.secondary }))
            .when(poll.multiple, |d| d.rounded_sm())
            .when(!poll.multiple, |d| d.rounded_full())
            .when(a.own, |d| d.bg(rgb(t.accent)).child(icon(Icon::Check, t.on_accent)));
        let label = div()
            .flex_1()
            .min_w_0()
            .whitespace_normal()
            .when(poll.ended && poll.is_leading(ix), |d| d.font_weight(FontWeight::BOLD))
            .child(a.text.clone());
        let row = div().relative().flex().items_center().gap_2().child(marker).child(label).when(shown, |d| {
            d.child(div().relative().flex_shrink_0().text_size(px(12.)).text_color(rgb(t.secondary))
                .child(format!("{} {}", a.votes, if a.votes == 1 { "voto" } else { "voti" }))
                .child(probe(PollPart::Votes(ix))))
        });
        let on_vote = on_vote.clone();
        div().id(("poll-option", row_ix * 32 + ix)).relative().flex().flex_col().gap_1().rounded_md().border_1()
            .border_color(if focused == Some(ix) { rgb(t.accent).into() } else { gpui::Hsla::transparent_black() })
            .when(!poll.ended, |d| d.cursor_pointer().on_click(move |_, window, cx| on_vote(ix, window, cx)))
            .child(row)
            .when(shown, |d| d.child(
                div().relative().ml(px(MARKER_PX + 8.)).h(px(4.)).rounded_full().bg(rgb(t.selected)).overflow_hidden()
                    .child(div().h_full().w(relative(poll.share(ix))).bg(rgb(t.accent)))
                    .child(probe(PollPart::Bar(ix)))))
            .child(probe(PollPart::Option(ix)))
    });
    div().relative().flex().flex_col().gap_2().min_w(px(200.)).max_w_full()
        .child(div().flex().items_start().gap_2()
            .child(icon(Icon::Poll, t.accent))
            .child(div().flex_1().min_w_0().whitespace_normal().font_weight(FontWeight::BOLD).child(poll.question.clone())))
        .children(options)
        .child(div().flex().justify_between().gap_2().text_size(px(12.)).text_color(rgb(t.secondary))
            .child(poll.footer())
            .when(poll.ended, |d| d.child("Chiuso"))
            .when(failed, |d| d.child(div().text_color(rgb(t.error)).child("Voto non inviato"))))
        .child(probe(PollPart::Bubble))
        .into_any_element()
}
