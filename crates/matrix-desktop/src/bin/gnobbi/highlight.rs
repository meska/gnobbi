//! The message a jump landed on (search hit, quoted message, «Ultimo letto») is lit for a moment, as in Telegram.
//! Pure state: the clock is passed in. The lit time starts at the FIRST draw of the bubble, not at the jump,
//! because the message may have to be loaded first.
use matrix_domain::EventId;
use std::time::{Duration, Instant};

const HOLD: f32 = 1.5;
const FADE: f32 = 1.0;
/// With reduced motion: full for this long, then off at once.
const HOLD_STILL: f32 = 2.5;
const STEP: Duration = Duration::from_millis(50);

/// 1.0 flat, then a linear fade to 0 (reduced motion: 1.0, then 0 with no fade).
pub fn opacity(elapsed: Duration, reduce_motion: bool) -> f32 {
    let t = elapsed.as_secs_f32();
    if reduce_motion {
        return if t < HOLD_STILL { 1. } else { 0. };
    }
    (1. - (t - HOLD) / FADE).clamp(0., 1.)
}

/// Byte ranges of `term` in `text`, ignoring case. Empty when lowercasing changes the byte length of any character (ranges would not map back).
pub fn term_ranges(text: &str, term: &str) -> Vec<std::ops::Range<usize>> {
    let (low, t) = (text.to_lowercase(), term.to_lowercase());
    if t.is_empty() || text.chars().any(|c| c.to_lowercase().map(char::len_utf8).sum::<usize>() != c.len_utf8()) {
        return Vec::new();
    }
    low.match_indices(&t).map(|(i, m)| i..i + m.len()).collect()
}

#[derive(Default)]
pub struct Highlight {
    target: Option<EventId>,
    /// The searched word, for the bubble to mark inside its text.
    term: Option<String>,
    drawn_at: Option<Instant>,
}

impl Highlight {
    /// Only for a room opened from a search hit or a jump; a plain open never calls this.
    pub fn set_target(&mut self, id: EventId, term: Option<String>) {
        *self = Self { target: Some(id), term, drawn_at: None };
    }

    /// The bubble of `id` was drawn: the first call starts the clock.
    pub fn mark_drawn(&mut self, id: &EventId, now: Instant) {
        if self.target.as_ref() == Some(id) {
            self.drawn_at.get_or_insert(now);
        }
    }

    /// 0 for any other event; the target is full until its clock runs.
    pub fn opacity_for(&self, id: &EventId, now: Instant, reduce_motion: bool) -> f32 {
        if self.target.as_ref() != Some(id) {
            return 0.;
        }
        self.drawn_at.map_or(1., |at| opacity(now.saturating_duration_since(at), reduce_motion))
    }

    /// The searched word while the effect lasts.
    pub fn term(&self) -> Option<&str> {
        self.term.as_deref().filter(|t| !t.is_empty())
    }

    /// Effect over (drawn and faded out): forget the target, so nothing is lit or repainted again.
    pub fn clear_if_over(&mut self, now: Instant, reduce_motion: bool) {
        if self.drawn_at.is_some() && self.repaint_in(now, reduce_motion).is_none() {
            *self = Self::default();
        }
    }

    /// The event to draw lit now, if any.
    pub fn lit(&self, now: Instant, reduce_motion: bool) -> Option<&EventId> {
        self.target.as_ref().filter(|id| self.opacity_for(id, now, reduce_motion) > 0.)
    }

    /// Repaint wanted after this long (`request_repaint_after`); None before the first draw and once the show is over.
    pub fn repaint_in(&self, now: Instant, reduce_motion: bool) -> Option<Duration> {
        let at = self.drawn_at?;
        let elapsed = now.saturating_duration_since(at);
        let end = Duration::from_secs_f32(if reduce_motion { HOLD_STILL } else { HOLD + FADE });
        let hold = Duration::from_secs_f32(if reduce_motion { HOLD_STILL } else { HOLD });
        // Flat phase: wake up when it ends, no frames in between; only the fade needs steps.
        (elapsed < end).then(|| if elapsed < hold { hold - elapsed } else { STEP })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration { Duration::from_millis(n) }
    fn id(s: &str) -> EventId { EventId::new(s) }

    #[test]
    fn fades_after_a_flat_start() {
        assert_eq!(opacity(ms(0), false), 1.);
        assert_eq!(opacity(ms(1000), false), 1.);
        assert!((opacity(ms(2000), false) - 0.5).abs() < 1e-4);
        assert!(opacity(ms(2499), false) < 0.01);
        assert_eq!(opacity(ms(2500), false), 0.);
        assert_eq!(opacity(ms(9000), false), 0.);
    }

    #[test]
    fn reduced_motion_has_no_fade() {
        assert_eq!(opacity(ms(2000), true), 1.);
        assert_eq!(opacity(ms(2400), true), 1.);
        assert_eq!(opacity(ms(2600), true), 0.);
    }

    #[test]
    fn term_ranges_ignore_case() {
        assert_eq!(term_ranges("Ciao ciao CIAO", "ciao"), vec![0..4, 5..9, 10..14]);
        assert!(term_ranges("abc", "").is_empty());
    }

    #[test]
    fn term_ranges_survive_hostile_case_mapping() {
        let t = "\u{130}\u{130}a\u{212A}";
        for r in term_ranges(t, "a") { assert!(t.is_char_boundary(r.start) && t.is_char_boundary(r.end)); }
        let t = "\u{130} foo \u{130}\u{212A}";
        assert!(term_ranges(t, "foo").iter().all(|r| &t[r.clone()] == "foo"));
    }

    #[test]
    fn repaints_only_while_fading() {
        let (mut h, t0) = (Highlight::default(), Instant::now());
        h.set_target(id("$a"), None);
        h.mark_drawn(&id("$a"), t0);
        assert_eq!(h.repaint_in(t0 + ms(500), false), Some(ms(1000)));
        assert_eq!(h.repaint_in(t0 + ms(1600), false), Some(STEP));
    }

    #[test]
    fn target_is_forgotten_when_over() {
        let (mut h, t0) = (Highlight::default(), Instant::now());
        h.set_target(id("$a"), Some("x".into()));
        h.clear_if_over(t0 + Duration::from_secs(60), false); // never drawn: stays
        assert!(h.term().is_some());
        h.mark_drawn(&id("$a"), t0);
        h.clear_if_over(t0 + ms(1000), false);
        assert!(h.term().is_some());
        h.clear_if_over(t0 + ms(2600), false);
        assert!(h.term().is_none() && h.lit(t0, false).is_none());
    }

    #[test]
    fn plain_open_lights_nothing() {
        let h = Highlight::default();
        let now = Instant::now();
        assert_eq!(h.opacity_for(&id("$a"), now, false), 0.);
        assert!(h.lit(now, false).is_none());
        assert!(h.repaint_in(now, false).is_none());
    }

    #[test]
    fn clock_starts_at_first_draw_only() {
        let mut h = Highlight::default();
        let t0 = Instant::now();
        h.set_target(id("$a"), Some("ciao".into()));
        // Not drawn yet (still loading): stays full, no repaint asked, however long it takes.
        assert_eq!(h.opacity_for(&id("$a"), t0 + Duration::from_secs(30), false), 1.);
        assert!(h.repaint_in(t0 + Duration::from_secs(30), false).is_none());
        // Another bubble does not start it.
        h.mark_drawn(&id("$b"), t0 + Duration::from_secs(30));
        assert!(h.repaint_in(t0 + Duration::from_secs(30), false).is_none());
        let t1 = t0 + Duration::from_secs(31);
        h.mark_drawn(&id("$a"), t1);
        h.mark_drawn(&id("$a"), t1 + Duration::from_secs(2)); // later draws do not restart it
        assert_eq!(h.opacity_for(&id("$a"), t1 + ms(1000), false), 1.);
        assert_eq!(h.opacity_for(&id("$b"), t1, false), 0.);
        assert!(h.repaint_in(t1 + ms(1000), false).is_some());
        assert!(h.repaint_in(t1 + ms(2500), false).is_none());
        assert!(h.lit(t1 + ms(2600), false).is_none());
    }
}
