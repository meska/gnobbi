//! Bounded reconnect pacing independent of runtimes, SDK and UI.
use std::time::Duration;

/// Exponential equal-jitter delays: 250–500ms initially, capped at 15–30s.
/// Seed is for desynchronization only, never for cryptography or identifiers.
pub struct Backoff {
    ceiling_ms: u64,
    entropy: u64,
}
impl Backoff {
    pub fn new(seed: u64) -> Self {
        Self {
            ceiling_ms: 500,
            entropy: seed.max(1),
        }
    }

    pub fn reset(&mut self) {
        self.ceiling_ms = 500;
    }

    pub fn next_delay(&mut self) -> Duration {
        self.entropy ^= self.entropy << 13;
        self.entropy ^= self.entropy >> 7;
        self.entropy ^= self.entropy << 17;
        let half = self.ceiling_ms / 2;
        let delay = half + self.entropy % (half + 1);
        self.ceiling_ms = (self.ceiling_ms * 2).min(30_000);
        Duration::from_millis(delay)
    }
}
