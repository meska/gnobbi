//! Poll bubble model (MSC3381), SDK- and UI-independent.

/// Read-only projection of a poll start plus its aggregated votes. Private user data: never log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Poll {
    pub question: String,
    /// Order of the start event, NOT by count.
    pub answers: Vec<PollAnswer>,
    /// >= 1 (0 from the wire is clamped to 1 by the engine).
    pub max_selections: u32,
    /// `max_selections > 1`.
    pub multiple: bool,
    pub disclosed: bool,
    pub ended: bool,
    /// Distinct users with a counted vote.
    pub voters: u32,
    /// Own poll and not ended.
    pub can_end: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollAnswer {
    pub id: String,
    pub text: String,
    pub votes: u32,
    pub own: bool,
}

impl Poll {
    /// Counts and bars are shown only then (undisclosed polls reveal them at the end).
    pub fn results_visible(&self) -> bool {
        self.disclosed || self.ended
    }

    pub fn own_answers(&self) -> Vec<String> {
        self.answers.iter().filter(|a| a.own).map(|a| a.id.clone()).collect()
    }

    /// Answers to send after activating `id`; `None` = nothing to send
    /// (ended, unknown id, same single vote, cap reached). `Some(vec![])` = withdraw every vote.
    pub fn toggle(&self, id: &str) -> Option<Vec<String>> {
        if self.ended || !self.answers.iter().any(|a| a.id == id) {
            return None;
        }
        let mut own = self.own_answers();
        if !self.multiple {
            return (own != [id]).then(|| vec![id.to_owned()]);
        }
        if let Some(at) = own.iter().position(|a| a == id) {
            own.remove(at);
        } else if own.len() >= self.max_selections as usize {
            return None;
        } else {
            own.push(id.to_owned());
        }
        Some(own)
    }

    /// Votes of option `ix` over distinct voters; 0.0 without voters or for a bad index.
    pub fn share(&self, ix: usize) -> f32 {
        match self.answers.get(ix) {
            Some(a) if self.voters > 0 => (a.votes as f32 / self.voters as f32).min(1.0),
            _ => 0.0,
        }
    }

    /// Option with the most votes (ties all lead), only if it has at least one.
    pub fn is_leading(&self, ix: usize) -> bool {
        let max = self.answers.iter().map(|a| a.votes).max().unwrap_or(0);
        self.answers.get(ix).is_some_and(|a| a.votes > 0 && a.votes == max)
    }
}

/// What the poll dialog sends. Private user data: never log.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PollDraft {
    pub question: String,
    pub answers: Vec<String>,
    pub multiple: bool,
    pub disclosed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollDraftError {
    EmptyQuestion,
    QuestionTooLong,
    TooFewAnswers,
    TooManyAnswers,
    /// Index of the offending answer in `answers`.
    EmptyAnswer(usize),
    AnswerTooLong(usize),
    /// Index of the later copy (case-insensitive after trimming).
    DuplicateAnswer(usize),
}

impl PollDraft {
    pub const MIN_ANSWERS: usize = 2;
    pub const MAX_ANSWERS: usize = 20;
    pub const MAX_CHARS: usize = 340;

    /// Checks the trimmed texts; lengths are in characters, not bytes.
    pub fn validate(&self) -> Result<(), PollDraftError> {
        use PollDraftError::*;
        let question = self.question.trim();
        if question.is_empty() {
            return Err(EmptyQuestion);
        }
        if question.chars().count() > Self::MAX_CHARS {
            return Err(QuestionTooLong);
        }
        if self.answers.len() < Self::MIN_ANSWERS {
            return Err(TooFewAnswers);
        }
        if self.answers.len() > Self::MAX_ANSWERS {
            return Err(TooManyAnswers);
        }
        let mut seen = std::collections::HashSet::new();
        for (ix, answer) in self.answers.iter().map(|a| a.trim()).enumerate() {
            if answer.is_empty() {
                return Err(EmptyAnswer(ix));
            }
            if answer.chars().count() > Self::MAX_CHARS {
                return Err(AnswerTooLong(ix));
            }
            if !seen.insert(answer.to_lowercase()) {
                return Err(DuplicateAnswer(ix));
            }
        }
        Ok(())
    }

    /// Plain-text copy for clients without polls: «question\n1. A\n2. B».
    pub fn fallback_text(&self) -> String {
        let mut text = self.question.trim().to_owned();
        for (ix, answer) in self.answers.iter().enumerate() {
            text.push_str(&format!("\n{}. {}", ix + 1, answer.trim()));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(multiple: bool, max: u32, votes: [(u32, bool); 3]) -> Poll {
        let answers = votes
            .iter()
            .enumerate()
            .map(|(i, (v, own))| PollAnswer { id: (i + 1).to_string(), text: format!("O{}", i + 1), votes: *v, own: *own })
            .collect();
        Poll {
            question: "Q".into(),
            answers,
            max_selections: max,
            multiple,
            disclosed: true,
            ended: false,
            voters: votes.iter().map(|v| v.0).sum(),
            can_end: false,
        }
    }
    fn ids(v: &[&str]) -> Option<Vec<String>> {
        Some(v.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn single_replaces_and_same_is_none() {
        let p = poll(false, 1, [(1, true), (0, false), (0, false)]);
        assert_eq!(p.toggle("2"), ids(&["2"]));
        assert_eq!(p.toggle("1"), None);
        assert_eq!(poll(false, 1, [(0, false); 3]).toggle("3"), ids(&["3"]));
        assert_eq!(p.toggle("9"), None);
    }

    #[test]
    fn multiple_adds_removes_caps_and_empties() {
        let p = poll(true, 2, [(1, true), (0, false), (0, false)]);
        assert_eq!(p.toggle("3"), ids(&["1", "3"]));
        let full = poll(true, 2, [(1, true), (1, true), (0, false)]);
        assert_eq!(full.toggle("3"), None);
        assert_eq!(full.toggle("1"), ids(&["2"]));
        assert_eq!(p.toggle("1"), Some(vec![]));
    }

    #[test]
    fn ended_cannot_toggle() {
        let mut p = poll(true, 3, [(0, false); 3]);
        p.ended = true;
        assert_eq!(p.toggle("1"), None);
    }

    #[test]
    fn share_and_leading() {
        let p = poll(false, 1, [(3, false), (1, false), (0, false)]);
        assert_eq!(p.share(0), 0.75);
        assert_eq!(p.share(9), 0.0);
        assert!(p.is_leading(0) && !p.is_leading(1) && !p.is_leading(2));
        let none = poll(false, 1, [(0, false); 3]);
        assert_eq!(none.share(0), 0.0);
        assert!(!none.is_leading(0));
        let tie = poll(false, 1, [(2, false), (2, false), (0, false)]);
        assert!(tie.is_leading(0) && tie.is_leading(1));
    }

    fn draft(answers: &[&str]) -> PollDraft {
        PollDraft {
            question: " Dove? ".into(),
            answers: answers.iter().map(|a| a.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn draft_errors_one_per_variant() {
        use PollDraftError::*;
        let check = |d: PollDraft, e| assert_eq!(d.validate(), Err(e));
        check(PollDraft { question: "  ".into(), ..draft(&["a", "b"]) }, EmptyQuestion);
        check(PollDraft { question: "é".repeat(341), ..draft(&["a", "b"]) }, QuestionTooLong);
        check(draft(&["a"]), TooFewAnswers);
        check(draft(&["a", "  "]), EmptyAnswer(1));
        check(draft(&["a", &"x".repeat(341)]), AnswerTooLong(1));
        check(draft(&["Si", "no", " SI "]), DuplicateAnswer(2));
        let many: Vec<String> = (0..21).map(|n| n.to_string()).collect();
        check(PollDraft { answers: many, ..draft(&[]) }, TooManyAnswers);
    }

    #[test]
    fn draft_valid_at_the_limits_and_fallback() {
        let long = "é".repeat(340); // 680 bytes, 340 characters
        let answers: Vec<String> = (0..20).map(|n| format!("{n}{}", &long[..long.len() - 2 * n.to_string().len()])).collect();
        let d = PollDraft { question: long.clone(), answers, ..Default::default() };
        assert_eq!(d.validate(), Ok(()));
        assert_eq!(draft(&[" A ", "B"]).fallback_text(), "Dove?\n1. A\n2. B");
    }
}
