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

#[derive(Default)]
pub struct Highlight {
    target: Option<EventId>,
    /// The searched word, for the bubble to mark inside its text.
    #[allow(dead_code)] // shortcut: kept for the draw task that marks the word; nothing reads it yet
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

    /// The event to draw lit now, if any.
    pub fn lit(&self, now: Instant, reduce_motion: bool) -> Option<&EventId> {
        self.target.as_ref().filter(|id| self.opacity_for(id, now, reduce_motion) > 0.)
    }

    /// Repaint wanted after this long (`request_repaint_after`); None before the first draw and once the show is over.
    pub fn repaint_in(&self, now: Instant, reduce_motion: bool) -> Option<Duration> {
        let at = self.drawn_at?;
        let elapsed = now.saturating_duration_since(at);
        let end = Duration::from_secs_f32(if reduce_motion { HOLD_STILL } else { HOLD + FADE });
        (elapsed < end).then(|| if reduce_motion { end - elapsed } else { STEP })
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
