//! Microphone capture (cpal) into mono 16-bit samples; encoding lives in `matrix_desktop::voice`.
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::sync::{Arc, Mutex};

pub struct Recorder {
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<i16>>>,
    rate: u32,
    max: usize,
    started: std::time::Instant,
}

/// Mono mix of interleaved frames, appended as i16.
fn push<T: SizedSample>(data: &[T], channels: usize, max: usize, out: &Mutex<Vec<i16>>)
where
    f32: FromSample<T>,
{
    let Ok(mut out) = out.lock() else { return };
    if out.len() >= max {
        return;
    }
    out.extend(data.chunks(channels).map(|frame| {
        let mean = frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32;
        (mean.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
    }));
}

impl Recorder {
    /// Starts capturing from the default input device; the error is the one line shown to the user.
    pub fn start(upload_limit: u64) -> Result<Self, &'static str> {
        const NO_MIC: &str = "Microfono non disponibile";
        let device = cpal::default_host().default_input_device().ok_or(NO_MIC)?;
        let config = device.default_input_config().map_err(|_| NO_MIC)?;
        let channels = usize::from(config.channels()).max(1);
        let rate = config.sample_rate();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let sink = samples.clone();
        // The note stops growing at 5 minutes or at the server's upload limit; the composer then ends it (`is_full`).
        let max = matrix_desktop::max_note_samples(rate, upload_limit);
        let err = |_| {};
        let format = config.sample_format();
        let config = config.into();
        let stream = match format {
            SampleFormat::F32 => device.build_input_stream(
                config,
                move |d: &[f32], _: &_| push(d, channels, max, &sink),
                err,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                config,
                move |d: &[i16], _: &_| push(d, channels, max, &sink),
                err,
                None,
            ),
            SampleFormat::I32 => device.build_input_stream(
                config,
                move |d: &[i32], _: &_| push(d, channels, max, &sink),
                err,
                None,
            ),
            _ => return Err(NO_MIC),
        }
        .map_err(|_| NO_MIC)?;
        stream.play().map_err(|_| NO_MIC)?;
        Ok(Self {
            _stream: stream,
            samples,
            rate,
            max,
            started: std::time::Instant::now(),
        })
    }

    /// The cap is reached: nothing more is captured.
    pub fn is_full(&self) -> bool {
        self.samples.lock().unwrap_or_else(|e| e.into_inner()).len() >= self.max
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Stops the stream and returns what was captured.
    pub fn finish(self) -> (Vec<i16>, u32) {
        let Self {
            _stream,
            samples,
            rate,
            ..
        } = self;
        drop(_stream);
        let samples = std::mem::take(&mut *samples.lock().unwrap_or_else(|e| e.into_inner()));
        (samples, rate)
    }
}
