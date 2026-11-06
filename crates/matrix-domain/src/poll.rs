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

fn votes_label(n: u32) -> String {
    format!("{n} {}", if n == 1 { "voto" } else { "voti" })
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

    /// Accessible name: «Si, 5 voti, selezionata»; counts only when results are visible.
    pub fn option_label(&self, ix: usize) -> String {
        let Some(a) = self.answers.get(ix) else {
            return String::new();
        };
        let mut label = a.text.clone();
        if self.results_visible() {
            label = format!("{label}, {}", votes_label(a.votes));
        }
        if a.own {
            label.push_str(", selezionata");
        }
        label
    }

    pub fn footer(&self) -> String {
        if self.results_visible() {
            format!("Basato su {}", votes_label(self.voters))
        } else {
            votes_label(self.voters)
        }
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

    #[test]
    fn labels() {
        let mut p = poll(false, 1, [(5, true), (1, false), (0, false)]);
        assert_eq!(p.option_label(0), "O1, 5 voti, selezionata");
        assert_eq!(p.option_label(1), "O2, 1 voto");
        p.disclosed = false;
        assert_eq!(p.option_label(0), "O1, selezionata");
        assert_eq!(p.option_label(1), "O2");
        assert_eq!(p.footer(), "6 voti");
        p.ended = true;
        assert_eq!(p.option_label(1), "O2, 1 voto");
        assert_eq!(p.footer(), "Basato su 6 voti");
        p.voters = 1;
        assert_eq!(p.footer(), "Basato su 1 voto");
        p.voters = 0;
        assert_eq!(p.footer(), "Basato su 0 voti");
    }
}
