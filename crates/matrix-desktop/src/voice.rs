//! Voice-note encoding: mono 16-bit samples to a WAV file plus duration and waveform.
use matrix_engine::sending::{Attachment, Voice};

/// Waveform points sent with the note (MSC1767 allows up to 120).
const WAVEFORM_POINTS: usize = 100;

/// Longest note, in seconds.
const MAX_SECONDS: usize = 300;

/// Most samples a note may hold: 5 minutes, and a WAV that fits `upload_limit` bytes.
pub fn max_note_samples(sample_rate: u32, upload_limit: u64) -> usize {
    (sample_rate as usize * MAX_SECONDS).min((upload_limit.saturating_sub(44) / 2) as usize)
}

/// Builds the attachment for a finished recording (cut to `max_note_samples`); `None` when nothing was captured.
// shortcut: uncompressed WAV (every Matrix client plays it); upgrade to Ogg/Opus when a pure-Rust encoder is vetted.
pub fn voice_attachment(samples: &[i16], sample_rate: u32, upload_limit: u64) -> Option<Attachment> {
    let samples = &samples[..samples.len().min(max_note_samples(sample_rate, upload_limit))];
    if samples.is_empty() || sample_rate == 0 {
        return None;
    }
    let bytes = u32::try_from(samples.len() * 2).ok()?;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    wav.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    let chunk = samples.len().div_ceil(WAVEFORM_POINTS);
    let waveform = samples
        .chunks(chunk)
        .map(|c| c.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0) as f32 / 32768.0)
        .collect();
    Some(Attachment {
        filename: "nota-vocale.wav".into(),
        data: wav,
        voice: Some(Voice {
            duration_ms: samples.len() as u64 * 1000 / u64::from(sample_rate),
            waveform,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::voice_attachment;

    const UP: u64 = 24 << 20;

    #[test]
    fn wav_has_header_duration_and_bounded_waveform() {
        assert!(voice_attachment(&[], 16000, UP).is_none());
        let samples: Vec<i16> = (0..32000)
            .map(|i| if i < 16000 { 0 } else { 16384 })
            .collect();
        let note = voice_attachment(&samples, 16000, UP).unwrap();
        assert_eq!(&note.data[..4], b"RIFF");
        assert_eq!(&note.data[8..16], b"WAVEfmt ");
        assert_eq!(note.data.len(), 44 + 64000);
        let voice = note.voice.unwrap();
        assert_eq!(voice.duration_ms, 2000);
        assert_eq!(voice.waveform.len(), 100);
        assert_eq!(voice.waveform[0], 0.0);
        assert_eq!(voice.waveform[99], 0.5);
    }

    #[test]
    fn waveform_stays_within_the_matrix_limit_for_any_length() {
        for len in [1usize, 99, 100, 101, 250, 1999, 100_001] {
            let samples: Vec<i16> = (0..len)
                .map(|i| if i % 2 == 0 { i16::MIN } else { 100 })
                .collect();
            let voice = voice_attachment(&samples, 48_000, UP).unwrap().voice.unwrap();
            assert!(
                (1..=100).contains(&voice.waveform.len()),
                "{len}: {}",
                voice.waveform.len()
            );
            assert!(
                voice.waveform.iter().all(|v| (0.0..=1.0).contains(v)),
                "{len}"
            );
            assert_eq!(
                voice.waveform[0], 1.0,
                "i16::MIN is full scale, no overflow"
            );
        }
    }

    #[test]
    fn degenerate_recordings_are_still_valid_files_or_refused() {
        assert!(voice_attachment(&[1, 2, 3], 0, UP).is_none());
        let tiny = voice_attachment(&[5], 48_000, UP).unwrap();
        assert_eq!((tiny.data.len(), tiny.voice.unwrap().duration_ms), (46, 0));
    }

    /// Tuwunel's default upload limit is 24 MiB; 5 minutes at 48 kHz mono 16-bit would be 27.5 MiB, and 192 kHz 110 MiB.
    #[test]
    fn longest_note_fits_the_smallest_default_upload_limit() {
        for rate in [48_000, 192_000] {
            let long = voice_attachment(&vec![0; rate * 300], rate as u32, UP).unwrap();
            assert!(long.data.len() as u64 <= UP, "{rate}: {} bytes", long.data.len());
        }
        // A bigger server limit keeps the full 5 minutes.
        let full = voice_attachment(&vec![0; 48_000 * 300], 48_000, 50 << 20).unwrap();
        assert_eq!(full.voice.unwrap().duration_ms, 300_000);
    }
}
