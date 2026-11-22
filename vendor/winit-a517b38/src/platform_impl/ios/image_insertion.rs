//! A single bounded native insertion, drained by the event-loop bridge.
pub(crate) const MAX_BYTES: usize = 25 * 1024 * 1024;
pub(crate) const MAX_DIMENSION: f64 = 8192.0;
pub(crate) const MAX_PIXELS: f64 = 8_000_000.0;

#[derive(Default)]
pub(crate) struct ImageInsertions {
    pending: Option<Result<(Vec<u8>, Option<String>), String>>,
}

impl ImageInsertions {
    pub(crate) fn push(&mut self, result: Result<(Vec<u8>, Option<String>), String>) -> bool {
        self.pending = Some(if self.pending.is_some() {
            Err("Only one attachment is supported".into())
        } else {
            result.and_then(|(bytes, description)| {
                if bytes.len() > MAX_BYTES {
                    Err("Image is too large".into())
                } else if bytes.is_empty() {
                    Err("Image cannot be attached".into())
                } else {
                    Ok((bytes, description))
                }
            })
        });
        self.pending.as_ref().is_some_and(Result::is_ok)
    }

    pub(crate) fn clear(&mut self) {
        self.pending = None;
    }

    pub(crate) fn take(&mut self) -> Vec<Result<(Vec<u8>, Option<String>), String>> {
        self.pending.take().into_iter().collect()
    }
}

pub(crate) fn valid_dimensions(width: f64, height: f64) -> bool {
    width.is_finite() && height.is_finite()
        && width > 0.0 && height > 0.0
        && width <= MAX_DIMENSION && height <= MAX_DIMENSION
        && width * height <= MAX_PIXELS
}
