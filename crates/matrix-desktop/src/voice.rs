//! Voice-note encoding: mono 16-bit samples to a WAV file plus duration and waveform.
use matrix_engine::sending::{Attachment, Voice};

/// Waveform points sent with the note (MSC1767 allows up to 120).
const WAVEFORM_POINTS: usize = 100;

/// Builds the attachment for a finished recording; `None` when nothing was captured.
// shortcut: uncompressed WAV (every Matrix client plays it); upgrade to Ogg/Opus when a pure-Rust encoder is vetted.
pub fn voice_attachment(samples: &[i16], sample_rate: u32) -> Option<Attachment> {
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

    #[test]
    fn wav_has_header_duration_and_bounded_waveform() {
        assert!(voice_attachment(&[], 16000).is_none());
        let samples: Vec<i16> = (0..32000).map(|i| if i < 16000 { 0 } else { 16384 }).collect();
        let note = voice_attachment(&samples, 16000).unwrap();
        assert_eq!(&note.data[..4], b"RIFF");
        assert_eq!(&note.data[8..16], b"WAVEfmt ");
        assert_eq!(note.data.len(), 44 + 64000);
        let voice = note.voice.unwrap();
        assert_eq!(voice.duration_ms, 2000);
        assert_eq!(voice.waveform.len(), 100);
        assert_eq!(voice.waveform[0], 0.0);
        assert_eq!(voice.waveform[99], 0.5);
    }
}
