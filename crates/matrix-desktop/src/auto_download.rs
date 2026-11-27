//! «Download automatico»: what the app fetches by itself, by type, network and declared size, and what stays behind a
//! download button. Pure decisions; the bytes are fetched by `LiveMedia` and kept in the engine's memory cache.
use matrix_engine::history::MediaKind;
use serde_json::{Value, json};

/// The largest size one kind is fetched up to (and in memory, held): a photo as the viewer reads it, the rest as the auto cache holds it.
pub fn cap(kind: Kind) -> u64 {
    match kind {
        Kind::Photo => matrix_engine::media::MAX_MEDIA_BYTES as u64,
        Kind::Video | Kind::File => (matrix_engine::media::MAX_SAVE_BYTES.min(matrix_engine::media::AUTO_BUDGET)) as u64,
    }
}

/// The choices of a row: «Always» means up to the cap, which is also enforced while the bytes arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Never,
    Mb1,
    Mb5,
    Mb15,
    Mb50,
    Always,
}

impl Limit {
    pub const ALL: [Limit; 6] = [Limit::Never, Limit::Mb1, Limit::Mb5, Limit::Mb15, Limit::Mb50, Limit::Always];
    fn name(self) -> &'static str {
        match self {
            Limit::Never => "never",
            Limit::Mb1 => "1",
            Limit::Mb5 => "5",
            Limit::Mb15 => "15",
            Limit::Mb50 => "50",
            Limit::Always => "always",
        }
    }
    fn parse(s: &str) -> Option<Limit> {
        Self::ALL.into_iter().find(|l| l.name() == s)
    }
    pub fn label(self) -> &'static str {
        match self {
            Limit::Never => crate::i18n::t("Never"),
            Limit::Mb1 => "1 MB",
            Limit::Mb5 => "5 MB",
            Limit::Mb15 => "15 MB",
            Limit::Mb50 => "50 MB",
            Limit::Always => crate::i18n::t("Always"),
        }
    }
    /// The size a step goes up to, in bytes (1 MB = 2^20, as sizes are shown); none for «Never» and «Always».
    fn bytes(self) -> Option<u64> {
        match self {
            Limit::Never | Limit::Always => None,
            mb => mb.name().parse::<u64>().ok().map(|n| n << 20),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Photo,
    Video,
    File,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Photo, Kind::Video, Kind::File];
    /// Voice notes are not in the settings: they stay as they are.
    pub fn of(media: MediaKind) -> Option<Kind> {
        match media {
            MediaKind::Image => Some(Kind::Photo),
            MediaKind::Video => Some(Kind::Video),
            MediaKind::File => Some(Kind::File),
            MediaKind::Audio => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Kind::Photo => "photo",
            Kind::Video => "video",
            Kind::File => "file",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Photo => crate::i18n::t("Photo"),
            Kind::Video => crate::i18n::t("Video"),
            Kind::File => crate::i18n::t("File"),
        }
    }
}

/// Where the data comes from. The desktop has one group, which is `Wifi`; a phone whose network is unknown counts as `Mobile`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Network {
    Mobile,
    #[default]
    Wifi,
}

impl Network {
    pub const ALL: [Network; 2] = [Network::Mobile, Network::Wifi];
    fn name(self) -> &'static str {
        match self {
            Network::Mobile => "mobile",
            Network::Wifi => "wifi",
        }
    }
}

/// What a row of the chat does about an attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Fetch it now (only for a row on screen).
    Fetch,
    /// Show the download button, with the size.
    Button,
    /// Neither: it is bigger than what can be held.
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoDownload {
    /// By kind, then by network: `[Mobile, Wifi]`.
    limits: [[Limit; 2]; 3],
}

impl Default for AutoDownload {
    /// Photos as they have always been (always); video and files only on Wi-Fi and desktop, up to 15 and 5 MB.
    fn default() -> Self {
        AutoDownload { limits: [[Limit::Always; 2], [Limit::Never, Limit::Mb15], [Limit::Never, Limit::Mb5]] }
    }
}

impl AutoDownload {
    pub fn limit(&self, kind: Kind, net: Network) -> Limit {
        self.limits[kind as usize][net as usize]
    }
    pub fn set(&mut self, kind: Kind, net: Network, limit: Limit) {
        self.limits[kind as usize][net as usize] = limit;
    }
    /// The setting alone: is a file of `size` (None = not declared) within what the user allows on this network.
    pub fn allows(&self, kind: Kind, net: Network, size: Option<u64>) -> bool {
        match self.limit(kind, net) {
            Limit::Never => false,
            Limit::Always => true,
            step => size.zip(step.bytes()).is_some_and(|(size, max)| size <= max),
        }
    }
    /// Fetch, button or nothing. A photo allowed is read as it always was (the bubble's picture), whatever its size; a video or a
    /// file is fetched whole, so it must also fit what memory holds.
    pub fn decide(&self, kind: Kind, net: Network, size: Option<u64>) -> Decision {
        let fits = size.is_none_or(|size| size <= cap(kind));
        match (self.allows(kind, net, size), kind) {
            (true, Kind::Photo) => Decision::Fetch,
            (true, _) if fits => Decision::Fetch,
            _ if fits => Decision::Button,
            _ => Decision::Nothing,
        }
    }
    /// Missing or unknown values keep the default.
    pub fn from_json(v: Option<&Value>) -> AutoDownload {
        let mut out = AutoDownload::default();
        for kind in Kind::ALL {
            for net in Network::ALL {
                if let Some(l) = v.and_then(|v| v.get(kind.name())?.get(net.name())?.as_str()).and_then(Limit::parse) {
                    out.set(kind, net, l);
                }
            }
        }
        out
    }
    pub fn to_json(&self) -> Value {
        let row = |kind: Kind| json!({ "mobile": self.limit(kind, Network::Mobile).name(), "wifi": self.limit(kind, Network::Wifi).name() });
        json!({ "photo": row(Kind::Photo), "video": row(Kind::Video), "file": row(Kind::File) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MB: u64 = 1 << 20;

    #[test]
    fn the_defaults_keep_photos_as_they_are_and_hold_video_and_files_back_on_mobile() {
        let d = AutoDownload::default();
        for net in Network::ALL {
            assert_eq!(d.decide(Kind::Photo, net, Some(8 * MB)), Decision::Fetch);
            assert_eq!(d.decide(Kind::Photo, net, None), Decision::Fetch, "a photo of unknown size is read as today");
        }
        assert_eq!(d.decide(Kind::File, Network::Wifi, Some(3 * MB)), Decision::Fetch);
        assert_eq!(d.decide(Kind::File, Network::Wifi, Some(5 * MB)), Decision::Fetch, "the limit itself is inside");
        assert_eq!(d.decide(Kind::File, Network::Wifi, Some(8 * MB)), Decision::Button);
        assert_eq!(d.decide(Kind::File, Network::Mobile, Some(1)), Decision::Button);
        assert_eq!(d.decide(Kind::Video, Network::Wifi, Some(15 * MB)), Decision::Fetch);
        assert_eq!(d.decide(Kind::Video, Network::Wifi, Some(15 * MB + 1)), Decision::Button);
        assert_eq!(d.decide(Kind::Video, Network::Mobile, Some(MB)), Decision::Button);
    }

    #[test]
    fn a_size_that_is_not_declared_only_passes_always_and_gets_a_button_otherwise() {
        let mut d = AutoDownload::default();
        assert_eq!(d.decide(Kind::File, Network::Wifi, None), Decision::Button);
        d.set(Kind::File, Network::Wifi, Limit::Always);
        assert_eq!(d.decide(Kind::File, Network::Wifi, None), Decision::Fetch);
    }

    #[test]
    fn never_and_every_step_follow_their_limit_by_kind_and_network() {
        for (limit, mb) in [(Limit::Mb1, 1), (Limit::Mb5, 5), (Limit::Mb15, 15), (Limit::Mb50, 50)] {
            for kind in Kind::ALL {
                for net in Network::ALL {
                    let mut d = AutoDownload::default();
                    d.set(kind, net, limit);
                    assert!(d.allows(kind, net, Some(mb * MB)), "{kind:?} {net:?} {mb}");
                    assert!(!d.allows(kind, net, Some(mb * MB + 1)), "{kind:?} {net:?} {mb}");
                    // The other cells did not move.
                    let other = if net == Network::Wifi { Network::Mobile } else { Network::Wifi };
                    assert_eq!(d.limit(kind, other), AutoDownload::default().limit(kind, other));
                }
            }
        }
        let mut d = AutoDownload::default();
        d.set(Kind::Photo, Network::Wifi, Limit::Never);
        assert!(!d.allows(Kind::Photo, Network::Wifi, Some(0)), "never is never, even for nothing");
        assert_eq!(d.decide(Kind::Photo, Network::Wifi, Some(10)), Decision::Button);
        assert_eq!(d.decide(Kind::Photo, Network::Mobile, Some(10)), Decision::Fetch);
    }

    #[test]
    fn what_memory_cannot_hold_gets_neither_fetch_nor_button() {
        let mut d = AutoDownload::default();
        d.set(Kind::File, Network::Wifi, Limit::Always);
        let over = cap(Kind::File) + 1;
        assert_eq!(d.decide(Kind::File, Network::Wifi, Some(over)), Decision::Nothing);
        assert_eq!(d.decide(Kind::File, Network::Wifi, Some(cap(Kind::File))), Decision::Fetch);
        assert_eq!(d.decide(Kind::Photo, Network::Wifi, Some(cap(Kind::Photo) + 1)), Decision::Fetch, "a photo is read as today");
        d.set(Kind::Photo, Network::Wifi, Limit::Never);
        assert_eq!(d.decide(Kind::Photo, Network::Wifi, Some(cap(Kind::Photo) + 1)), Decision::Nothing);
    }

    #[test]
    fn voice_notes_are_not_a_kind() {
        assert_eq!(Kind::of(MediaKind::Audio), None);
        assert_eq!(Kind::of(MediaKind::File), Some(Kind::File));
    }

    #[test]
    fn the_saved_form_round_trips_and_a_missing_or_bad_value_keeps_the_default() {
        let mut d = AutoDownload::default();
        d.set(Kind::Video, Network::Mobile, Limit::Mb50);
        d.set(Kind::File, Network::Wifi, Limit::Never);
        assert_eq!(AutoDownload::from_json(Some(&d.to_json())), d);
        assert_eq!(AutoDownload::from_json(None), AutoDownload::default());
        let bad = json!({ "photo": { "mobile": "3 parsecs", "wifi": 7 }, "video": "x" });
        assert_eq!(AutoDownload::from_json(Some(&bad)), AutoDownload::default());
    }
}
