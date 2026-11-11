//! In-app playback of a decoded voice note: a sample cursor the cpal callback pulls from.
//! Tests drive the cursor directly, so Play/Pause and the progress need no audio device.
use std::sync::{Arc, Mutex};

/// Cursor over decoded mono samples. `rate` is the note's own (Opus decodes at 48 kHz).
pub struct Player {
    samples: Vec<f32>,
    rate: u32,
    position: usize,
    playing: bool,
}

impl Player {
    /// A freshly decoded note starts playing from its first sample.
    pub fn new(samples: Vec<f32>, rate: u32) -> Self {
        let playing = !samples.is_empty();
        Self {
            samples,
            rate: rate.max(1),
            position: 0,
            playing,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn finished(&self) -> bool {
        self.position >= self.samples.len()
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Play/Pause. A finished note starts again from the beginning.
    pub fn toggle(&mut self) {
        if self.finished() {
            self.position = 0;
            self.playing = !self.samples.is_empty();
        } else {
            self.playing = !self.playing;
        }
    }

    /// Along the note, 0..=1.
    pub fn progress(&self) -> f32 {
        if self.samples.is_empty() {
            0.0
        } else {
            (self.position as f32 / self.samples.len() as f32).clamp(0.0, 1.0)
        }
    }

    /// The note's own length: the bubble falls back to it when the event declares no duration.
    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 * 1000 / u64::from(self.rate)
    }

    /// Fill `out` with the next mono samples, advancing the cursor; silence while paused or
    /// finished. Returns how many samples of the note were written.
    pub fn pull(&mut self, out: &mut [f32]) -> usize {
        if !self.playing {
            out.fill(0.0);
            return 0;
        }
        let n = out.len().min(self.samples.len() - self.position);
        out[..n].copy_from_slice(&self.samples[self.position..self.position + n]);
        out[n..].fill(0.0);
        self.position += n;
        if self.position >= self.samples.len() {
            self.playing = false;
        }
        n
    }
}

/// The cpal output stream plus the shared cursor: dropping it stops the sound.
pub struct AudioOutput {
    _stream: cpal::Stream,
    player: Arc<Mutex<Player>>,
}

const NO_OUT: &str = "Riproduzione audio non disponibile";

impl AudioOutput {
    /// Open the default output at the note's own rate and start pulling. One line when there
    /// is no device or no supported sample format.
    pub fn start(player: Player) -> Result<Self, &'static str> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let rate = player.rate();
        let player = Arc::new(Mutex::new(player));
        let device = cpal::default_host().default_output_device().ok_or(NO_OUT)?;
        let supported = device.default_output_config().map_err(|_| NO_OUT)?;
        let channels = usize::from(supported.channels()).max(1);
        let format = supported.sample_format();
        let mut config: cpal::StreamConfig = supported.into();
        // Ask for the note's own rate so nothing is resampled; a device that refuses fails here.
        config.sample_rate = rate;
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, config, channels, player.clone()),
            cpal::SampleFormat::I16 => build::<i16>(&device, config, channels, player.clone()),
            cpal::SampleFormat::I32 => build::<i32>(&device, config, channels, player.clone()),
            _ => Err(NO_OUT),
        }?;
        stream.play().map_err(|_| NO_OUT)?;
        Ok(Self {
            _stream: stream,
            player,
        })
    }

    /// The shared cursor: the UI reads the playhead and toggles Play/Pause through it.
    pub fn player(&self) -> Arc<Mutex<Player>> {
        Arc::clone(&self.player)
    }
}

fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    player: Arc<Mutex<Player>>,
) -> Result<cpal::Stream, &'static str> {
    use cpal::traits::DeviceTrait;
    let mut mono: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels;
                mono.resize(frames, 0.0);
                let mut player = player.lock().unwrap_or_else(|e| e.into_inner());
                player.pull(&mut mono);
                for (i, frame) in data.chunks_mut(channels).enumerate() {
                    let sample = T::from_sample(mono[i]);
                    frame.fill(sample);
                }
            },
            |_| {},
            None,
        )
        .map_err(|_| NO_OUT)
}

#[cfg(test)]
mod tests {
    use super::Player;

    fn note(samples: usize) -> Player {
        Player::new(vec![0.5; samples], 1_000)
    }

    #[test]
    fn a_new_note_plays_and_pause_resumes_from_where_it_stopped() {
        let mut player = note(10);
        assert!(player.is_playing());
        let mut out = [0.0f32; 4];
        assert_eq!(player.pull(&mut out), 4);
        assert_eq!(out, [0.5; 4]);
        player.toggle();
        assert!(!player.is_playing());
        // Paused: silence, and the cursor does not move.
        assert_eq!(player.pull(&mut out), 0);
        assert_eq!(out, [0.0; 4]);
        assert!(
            (player.progress() - 0.4).abs() < f32::EPSILON,
            "the cursor did not move"
        );
        player.toggle();
        assert_eq!(player.pull(&mut out), 4);
        assert!((player.progress() - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn playback_stops_at_the_end_and_a_new_toggle_restarts_it() {
        let mut player = note(4);
        let mut out = [0.0f32; 8];
        assert_eq!(player.pull(&mut out), 4);
        assert!(!player.is_playing(), "the end of the note stops the sound");
        assert!(player.finished());
        assert_eq!(player.progress(), 1.0);
        assert_eq!(player.pull(&mut out), 0);
        player.toggle();
        assert!(
            player.is_playing(),
            "toggle after the end replays from the start"
        );
        assert_eq!(player.progress(), 0.0);
    }

    #[test]
    fn progress_follows_the_cursor_and_stays_bounded() {
        let mut player = note(100);
        assert_eq!(player.progress(), 0.0);
        let mut out = [0.0f32; 25];
        player.pull(&mut out);
        assert!(
            (player.progress() - 0.25).abs() < f32::EPSILON,
            "{}",
            player.progress()
        );
        assert_eq!(player.duration_ms(), 100);
        for _ in 0..10 {
            player.pull(&mut out);
        }
        assert_eq!(player.progress(), 1.0);
        assert!(player.progress() <= 1.0);
    }

    #[test]
    fn an_empty_note_never_plays_and_never_panics() {
        let mut player = Player::new(Vec::new(), 48_000);
        assert!(!player.is_playing());
        assert!(player.finished());
        assert_eq!(player.progress(), 0.0);
        let mut out = [0.0f32; 4];
        assert_eq!(player.pull(&mut out), 0);
        player.toggle();
        assert!(!player.is_playing(), "nothing to play");
        assert_eq!(player.duration_ms(), 0);
    }

    #[test]
    fn a_zero_rate_is_treated_as_one_so_the_labels_never_divide_by_zero() {
        let player = Player::new(vec![0.0; 10], 0);
        assert_eq!(player.rate(), 1);
        assert_eq!(player.duration_ms(), 10_000);
    }
}
