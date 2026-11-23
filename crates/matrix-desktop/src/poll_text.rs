//! Localized poll labels for the timeline and accessibility.
use matrix_domain::Poll;
use crate::i18n;

fn votes(n: u32) -> String {
    i18n::plural(n.into(), "{n} vote", "{n} votes")
}

pub fn option_label(poll: &Poll, ix: usize) -> String {
    let Some(answer) = poll.answers.get(ix) else { return String::new() };
    let mut label = answer.text.clone();
    if poll.results_visible() { label.push_str(&format!(", {}", votes(answer.votes))); }
    if answer.own { label.push_str(&format!(", {}", i18n::t("selected"))); }
    label
}

pub fn footer(poll: &Poll) -> String {
    let votes = votes(poll.voters);
    if poll.results_visible() { i18n::t("Based on {votes}").replace("{votes}", &votes) } else { votes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::PollAnswer;

    #[test]
    fn labels_follow_language_and_visibility() {
        let mut p = Poll { question: "Q".into(), answers: vec![PollAnswer { id: "a".into(), text: "A".into(), votes: 1, own: true }], max_selections: 1, multiple: false, disclosed: true, ended: false, voters: 1, can_end: false };
        i18n::apply(i18n::Preference::Fixed("en"), None);
        assert_eq!(option_label(&p, 0), "A, 1 vote, selected");
        assert_eq!(footer(&p), "Based on 1 vote");
        i18n::apply(i18n::Preference::Fixed("it"), None);
        assert_eq!(option_label(&p, 0), "A, 1 voto, selezionata");
        p.voters = 2;
        assert_eq!(footer(&p), "Basato su 2 voti");
        p.disclosed = false;
        assert_eq!(option_label(&p, 0), "A, selezionata");
        assert_eq!(footer(&p), "2 voti");
        p.ended = true;
        assert_eq!(footer(&p), "Basato su 2 voti");
        assert_eq!(option_label(&p, 9), "");
    }
}
