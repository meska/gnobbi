//! Logical-pixel geometry shared by both native scrolling panes.

/// Measured row heights plus a running-mean estimate for rows not laid out yet.
/// Two Fenwick sums keep prefix lookups logarithmic without constructing rows.
#[derive(Clone)]
pub struct VirtualScrollExtent {
    heights: Vec<f32>,
    sums: Vec<f32>,
    counts: Vec<usize>,
    sum: f32,
    measured: usize,
}
impl VirtualScrollExtent {
    pub fn new(count: usize) -> Self {
        Self {
            heights: vec![0.; count],
            sums: vec![0.; count + 1],
            counts: vec![0; count + 1],
            sum: 0.,
            measured: 0,
        }
    }
    pub fn measure(&mut self, index: usize, height: f32) {
        if index >= self.heights.len() || !height.is_finite() || height <= 0. {
            return;
        }
        let old = self.heights[index];
        let fresh = usize::from(old == 0.);
        self.heights[index] = height;
        self.sum += height - old;
        self.measured += fresh;
        let mut i = index + 1;
        while i < self.sums.len() {
            self.sums[i] += height - old;
            self.counts[i] += fresh;
            i += i.isolate_lowest_one();
        }
    }
    fn average(&self) -> f32 {
        if self.measured == 0 {
            100.
        } else {
            self.sum / self.measured as f32
        }
    }
    pub fn total_height(&self) -> f32 {
        self.offset_for_item(self.heights.len(), 0.)
    }
    pub fn offset_for_item(&self, index: usize, offset: f32) -> f32 {
        let count = index.min(self.heights.len());
        let (mut sum, mut measured, mut i) = (0., 0, count);
        while i > 0 {
            sum += self.sums[i];
            measured += self.counts[i];
            i &= i - 1;
        }
        sum + (count - measured) as f32 * self.average() + offset
    }
    pub fn item_for_offset(&self, offset: f32) -> (usize, f32) {
        let offset = offset.clamp(0., self.total_height());
        let (mut low, mut high) = (0, self.heights.len());
        while low < high {
            let mid = (low + high).div_ceil(2);
            if self.offset_for_item(mid, 0.) <= offset {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        (low, offset - self.offset_for_item(low, 0.))
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarGeometry {
    pub thumb_top: f32,
    pub thumb_length: f32,
    travel: f32,
    max_offset: f32,
}

impl ScrollbarGeometry {
    pub fn new(viewport: f32, content: f32, offset: f32) -> Option<Self> {
        if !viewport.is_finite()
            || !content.is_finite()
            || !offset.is_finite()
            || viewport <= 0.
            || content <= viewport
        {
            return None;
        }
        let max_offset = content - viewport;
        let thumb_length = (viewport * viewport / content)
            .max(28.)
            .min((viewport - 1.).max(viewport / 2.));
        let travel = viewport - thumb_length;
        Some(Self {
            thumb_top: offset.clamp(0., max_offset) / max_offset * travel,
            thumb_length,
            travel,
            max_offset,
        })
    }

    pub fn offset_for_thumb(self, top: f32) -> f32 {
        top.clamp(0., self.travel) / self.travel * self.max_offset
    }
}
