use collections::HashMap;

#[derive(Debug, Hash, PartialEq, Eq)]
pub(crate) enum SerialKind {
    DataDevice,
    InputMethod,
    MouseEnter,
    MousePress,
    KeyPress,
}

#[derive(Debug)]
struct SerialData {
    serial: u32,
}

impl SerialData {
    fn new(value: u32) -> Self {
        Self { serial: value }
    }
}

#[derive(Debug)]
/// Helper for tracking of different serial kinds.
pub(crate) struct SerialTracker {
    serials: HashMap<SerialKind, SerialData>,
}

impl SerialTracker {
    pub fn new() -> Self {
        Self {
            serials: HashMap::default(),
        }
    }

    pub fn update(&mut self, kind: SerialKind, value: u32) {
        self.serials.insert(kind, SerialData::new(value));
    }

    /// Returns the latest tracked serial of the provided [`SerialKind`]
    ///
    /// Will return 0 if not tracked.
    pub fn get(&self, kind: SerialKind) -> u32 {
        self.serials
            .get(&kind)
            .map(|serial_data| serial_data.serial)
            .unwrap_or(0)
    }

    /// Serial of the latest key or button event: `set_selection` accepts either,
    /// a pointer click must not be rejected for an older key serial.
    pub fn latest_input(&self) -> u32 {
        let (key, mouse) = (self.get(SerialKind::KeyPress), self.get(SerialKind::MousePress));
        if (mouse.wrapping_sub(key) as i32) > 0 { mouse } else { key }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_input_is_newest_of_key_and_mouse_even_across_wraparound() {
        let mut t = SerialTracker::new();
        t.update(SerialKind::KeyPress, 5);
        t.update(SerialKind::MousePress, 9);
        assert_eq!(t.latest_input(), 9);
        t.update(SerialKind::KeyPress, 12);
        assert_eq!(t.latest_input(), 12);
        t.update(SerialKind::KeyPress, u32::MAX - 1);
        t.update(SerialKind::MousePress, 3);
        assert_eq!(t.latest_input(), 3);
    }
}
