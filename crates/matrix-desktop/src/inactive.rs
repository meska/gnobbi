//! Pure filter behind «Disconnetti inattivi…»: which devices are idle past a threshold.
use matrix_engine::verification::VerificationDevice;

const DAY_MS: u64 = 86_400_000;

/// Idle thresholds. Months are FIXED DAYS (30 / 90 / 180 / 365), not calendar months:
/// the cut-off is then plain integer arithmetic, identical on every platform and
/// timezone, and a day of drift is irrelevant for «not used for ages».
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InactiveThreshold {
    OneMonth,
    ThreeMonths,
    #[default]
    SixMonths,
    OneYear,
}

impl InactiveThreshold {
    pub const fn millis(self) -> u64 {
        DAY_MS
            * match self {
                Self::OneMonth => 30,
                Self::ThreeMonths => 90,
                Self::SixMonths => 180,
                Self::OneYear => 365,
            }
    }
}

impl InactiveThreshold {
    /// Selector text; the selector cycles in this order.
    pub const fn label(self) -> &'static str {
        match self {
            Self::OneMonth => "1 mese",
            Self::ThreeMonths => "3 mesi",
            Self::SixMonths => "6 mesi",
            Self::OneYear => "1 anno",
        }
    }
    pub const fn next(self) -> Self {
        match self {
            Self::OneMonth => Self::ThreeMonths,
            Self::ThreeMonths => Self::SixMonths,
            Self::SixMonths => Self::OneYear,
            Self::OneYear => Self::OneMonth,
        }
    }
}

pub struct InactiveDevices<'a> {
    /// Devices to sign out, in input order.
    pub devices: Vec<&'a VerificationDevice>,
    /// Devices left out only because their last activity is unknown (for the popup).
    pub unknown_activity: usize,
}

/// `now_ms` is the injected clock (ms since the Unix epoch). A device is included only if
/// `last_seen_ts < now_ms - threshold` (exactly on the cut-off is NOT included). The current
/// device is never included or counted. Missing or 0 last activity counts as unknown.
pub fn inactive_devices<'a>(
    devices: &'a [VerificationDevice],
    own_device_id: Option<&str>,
    threshold: InactiveThreshold,
    now_ms: u64,
) -> InactiveDevices<'a> {
    let cutoff = now_ms.saturating_sub(threshold.millis());
    let mut out = InactiveDevices { devices: Vec::new(), unknown_activity: 0 };
    for d in devices.iter().filter(|d| Some(d.device_id.as_str()) != own_device_id) {
        match d.last_seen_ts {
            None | Some(0) => out.unknown_activity += 1,
            Some(t) if t < cutoff => out.devices.push(d),
            Some(_) => {}
        }
    }
    out
}

/// Visibility of «Disconnetti inattivi…»: some device is idle past the smallest threshold.
pub fn has_inactive_devices(devices: &[VerificationDevice], own_device_id: Option<&str>, now_ms: u64) -> bool {
    !inactive_devices(devices, own_device_id, InactiveThreshold::OneMonth, now_ms).devices.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use InactiveThreshold::*;

    const NOW: u64 = 400 * DAY_MS;

    fn dev(id: &str, ts: Option<u64>) -> VerificationDevice {
        VerificationDevice {
            user_id: "@a:x".into(),
            device_id: id.into(),
            verified: false,
            display_name: None,
            last_seen_ts: ts,
            last_seen_ip: None,
        }
    }
    fn ids<'a>(r: &'a InactiveDevices) -> Vec<&'a str> {
        r.devices.iter().map(|d| d.device_id.as_str()).collect()
    }

    #[test]
    fn default_is_six_months() {
        assert_eq!(InactiveThreshold::default(), SixMonths);
    }

    #[test]
    fn boundary_exact_second_before_and_after() {
        let cut = NOW - SixMonths.millis();
        let list = [dev("BEFORE", Some(cut - 1000)), dev("EXACT", Some(cut)), dev("AFTER", Some(cut + 1000))];
        let r = inactive_devices(&list, None, SixMonths, NOW);
        assert_eq!(ids(&r), ["BEFORE"]);
        assert_eq!(r.unknown_activity, 0);
    }

    #[test]
    fn every_threshold_cuts_at_its_own_age() {
        let list: Vec<_> = [(29, "D29"), (31, "D31"), (89, "D89"), (91, "D91"), (179, "D179"), (181, "D181"), (364, "D364"), (366, "D366")]
            .iter()
            .map(|(d, id)| dev(id, Some(NOW - d * DAY_MS)))
            .collect();
        let got = |t| inactive_devices(&list, None, t, NOW).devices.len();
        assert_eq!((got(OneMonth), got(ThreeMonths), got(SixMonths), got(OneYear)), (7, 5, 3, 1));
    }

    #[test]
    fn unknown_activity_is_excluded_and_counted() {
        let list = [dev("NONE", None), dev("ZERO", Some(0)), dev("OLD", Some(1))];
        let r = inactive_devices(&list, None, OneYear, NOW);
        assert_eq!(ids(&r), ["OLD"]);
        assert_eq!(r.unknown_activity, 2);
    }

    #[test]
    fn current_device_is_never_included_nor_counted() {
        let list = [dev("ME", Some(1)), dev("ME2", None), dev("OLD", Some(1))];
        let r = inactive_devices(&list, Some("ME"), OneMonth, NOW);
        assert_eq!(ids(&r), ["OLD"]);
        assert_eq!(r.unknown_activity, 1);
        let only_me = [dev("ME", None)];
        let r = inactive_devices(&only_me, Some("ME"), OneMonth, NOW);
        assert_eq!((r.devices.len(), r.unknown_activity), (0, 0));
    }

    #[test]
    fn empty_list_and_clock_before_threshold() {
        let r = inactive_devices(&[], None, SixMonths, NOW);
        assert!(r.devices.is_empty() && r.unknown_activity == 0);
        let young = [dev("A", Some(5))];
        assert!(inactive_devices(&young, None, OneYear, 10).devices.is_empty());
    }

    #[test]
    fn action_visible_only_with_a_device_idle_over_a_month() {
        let recent = [dev("A", Some(NOW - 29 * DAY_MS)), dev("N", None), dev("ME", Some(1))];
        assert!(!has_inactive_devices(&recent, Some("ME"), NOW));
        assert!(!has_inactive_devices(&[], None, NOW));
        let old = [dev("A", Some(NOW - 31 * DAY_MS))];
        assert!(has_inactive_devices(&old, None, NOW));
    }
}
