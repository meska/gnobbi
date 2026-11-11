//! Ogg/Opus decoding in pure Rust (opus-pure), plus the waveform of the same samples.
//!
//! One decode feeds both the player and the bubble's waveform: callers keep the
//! [`DecodedAudio`] and take the waveform from [`waveform`], never a second pass.
use crate::media::MediaError;
use opus_pure::{MAX_PACKET_SAMPLES, OggOpusReader, Trim};

/// Opus decodes to 8/12/16/24/48 kHz only; 48 kHz needs no resampling for playback.
pub const DECODE_RATE: u32 = 48_000;

/// Waveform points the bubble draws (MSC1767 allows up to 120; `voice.rs` sends 100).
pub const WAVEFORM_POINTS: usize = 100;

/// Mono samples at [`DECODE_RATE`]. Stereo and multichannel sources are mixed down.
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Ogg/Opus bytes to mono samples. Empty, truncated, non-Ogg or undecodable input is
/// a failure, never a panic.
pub fn decode_ogg_opus(bytes: &[u8]) -> Result<DecodedAudio, MediaError> {
    if bytes.is_empty() {
        return Err(MediaError::UnsupportedType);
    }
    let mut reader =
        OggOpusReader::new(std::io::Cursor::new(bytes)).map_err(|_| MediaError::UnsupportedType)?;
    let head = reader.head().clone();
    let channels = usize::from(head.channel_count);
    if channels == 0 {
        return Err(MediaError::UnsupportedType);
    }
    let rate = DECODE_RATE as i32;
    let mut decoder = head.decoder(rate).map_err(|_| MediaError::Failed)?;
    let mut trim = Trim::new(&head, rate, channels).map_err(|_| MediaError::Failed)?;
    let mut block = vec![0.0f32; MAX_PACKET_SAMPLES * channels];
    let mut samples: Vec<f32> = Vec::new();
    for packet in reader.packets() {
        let packet = packet.map_err(|_| MediaError::Failed)?;
        let n = decoder
            .decode(&packet.data, MAX_PACKET_SAMPLES, &mut block)
            .map_err(|_| MediaError::Failed)?;
        let kept = trim.keep(&packet, &block[..n * channels]);
        if channels == 1 {
            samples.extend_from_slice(kept);
        } else {
            samples.extend(
                kept.chunks(channels)
                    .map(|frame| frame.iter().sum::<f32>() / channels as f32),
            );
        }
    }
    if samples.is_empty() {
        return Err(MediaError::Failed);
    }
    Ok(DecodedAudio {
        samples,
        sample_rate: DECODE_RATE,
    })
}

/// MSC1767 waveform: `points` peaks in 0..=1, the same shape `voice.rs` writes.
pub fn waveform(samples: &[f32], points: usize) -> Vec<f32> {
    if samples.is_empty() || points == 0 {
        return Vec::new();
    }
    let chunk = samples.len().div_ceil(points);
    samples
        .chunks(chunk)
        .map(|c| c.iter().fold(0.0f32, |m, s| m.max(s.abs())).min(1.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{DECODE_RATE, decode_ogg_opus, waveform};

    /// A real Ogg/Opus stream: the crate encodes, so no fixture file is needed.
    fn sample_ogg(rate: u32, seconds: f32, amplitude: f32) -> Vec<u8> {
        use opus_pure::{Application, MAX_PACKET_BYTES, OggOpusWriter, OpusEncoder, OpusHead};
        let frame = (rate / 50) as usize; // 20 ms
        let total = ((rate as f32 * seconds) as usize).div_ceil(frame) * frame;
        let pcm: Vec<f32> = (0..total)
            .map(|i| amplitude * (i as f32 * 0.05).sin())
            .collect();
        let mut encoder = OpusEncoder::new(rate as i32, 1, Application::Audio).unwrap();
        let head = OpusHead::for_encoder(&encoder, rate);
        let mut writer = OggOpusWriter::new(Vec::new(), head).unwrap();
        let mut packet = vec![0u8; MAX_PACKET_BYTES];
        for block in pcm.chunks_exact(frame) {
            let n = encoder.encode(block, frame, &mut packet).unwrap();
            writer.write_packet(&packet[..n]).unwrap();
        }
        writer.finish().unwrap()
    }

    #[test]
    fn broken_or_empty_input_fails_without_panicking() {
        assert!(decode_ogg_opus(&[]).is_err());
        assert!(decode_ogg_opus(b"not an ogg file at all").is_err());
        assert!(decode_ogg_opus(&[0u8; 512]).is_err());
        // A truncated real stream: the Ogg header alone is not decodable audio.
        let ogg = sample_ogg(48_000, 0.2, 0.4);
        assert!(decode_ogg_opus(&ogg[..16]).is_err());
    }

    #[test]
    fn a_real_note_decodes_to_mono_at_the_playback_rate() {
        let ogg = sample_ogg(48_000, 0.5, 0.5);
        let audio = decode_ogg_opus(&ogg).unwrap();
        assert_eq!(audio.sample_rate, DECODE_RATE);
        // ~0.5 s at 48 kHz, allowing for Opus pre-skip and frame padding.
        let secs = audio.samples.len() as f32 / DECODE_RATE as f32;
        assert!((0.4..0.6).contains(&secs), "{secs}");
        assert!(audio.samples.iter().all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn waveform_is_bounded_and_has_one_point_per_chunk() {
        let samples: Vec<f32> = (0..48_000).map(|i| (i as f32 / 100.0).sin()).collect();
        for points in [1usize, 10, 100, 500] {
            let w = waveform(&samples, points);
            assert!((1..=points).contains(&w.len()), "{points}: {}", w.len());
            assert!(w.iter().all(|v| (0.0..=1.0).contains(v)));
        }
        assert!(waveform(&[], 10).is_empty());
        assert!(waveform(&samples, 0).is_empty());
    }

    #[test]
    fn the_waveform_tracks_the_boom_of_the_note() {
        // First second quiet, second second loud: the two halves must differ.
        let samples: Vec<f32> = (0..96_000)
            .map(|i| if i < 48_000 { 0.0 } else { 0.8 })
            .collect();
        let w = waveform(&samples, 2);
        assert_eq!(w.len(), 2);
        assert!(w[0] < 0.05, "{}", w[0]);
        assert!(w[1] > 0.7, "{}", w[1]);
    }
}
