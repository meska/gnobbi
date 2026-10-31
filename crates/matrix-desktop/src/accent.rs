//! Main colour («colore principale»): one tone per theme family, applied to every `tokens()` call.
use crate::Theme;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Blue,
    Teal,
    Green,
    Orange,
    Red,
    Pink,
    Purple,
}

impl Accent {
    pub const ALL: [Accent; 7] = [
        Accent::Blue,
        Accent::Teal,
        Accent::Green,
        Accent::Orange,
        Accent::Red,
        Accent::Pink,
        Accent::Purple,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Accent::Blue => "blue",
            Accent::Teal => "teal",
            Accent::Green => "green",
            Accent::Orange => "orange",
            Accent::Red => "red",
            Accent::Pink => "pink",
            Accent::Purple => "purple",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Accent::Blue => "Blu",
            Accent::Teal => "Verde acqua",
            Accent::Green => "Verde",
            Accent::Orange => "Arancione",
            Accent::Red => "Rosso",
            Accent::Pink => "Rosa",
            Accent::Purple => "Viola",
        }
    }

    /// Tone for light or dark themes; `None` for blue, which keeps each theme's own accent.
    pub fn tone(self, dark: bool) -> Option<u32> {
        Some(match (self, dark) {
            (Accent::Blue, _) => return None,
            (Accent::Teal, false) => 0x0b6b63,
            (Accent::Teal, true) => 0x5fd3c4,
            (Accent::Green, false) => 0x1b6e2d,
            (Accent::Green, true) => 0x84dc90,
            (Accent::Orange, false) => 0x9a4400,
            (Accent::Orange, true) => 0xffb067,
            (Accent::Red, false) => 0xb3261e,
            (Accent::Red, true) => 0xff9a94,
            (Accent::Pink, false) => 0xa8205f,
            (Accent::Pink, true) => 0xff8fc0,
            (Accent::Purple, false) => 0x6a3db0,
            (Accent::Purple, true) => 0xc4a5ff,
        })
    }

    /// Saved value or an OS accent name (GNOME `accent-color`); unknown names give `None`.
    pub fn parse(s: &str) -> Option<Accent> {
        match s {
            "yellow" => Some(Accent::Orange),
            "slate" => Some(Accent::Blue),
            _ => Self::ALL.into_iter().find(|a| a.name() == s),
        }
    }
}

/// The accent the OS uses, mapped onto our palette; `None` when it cannot be read.
pub fn system_accent() -> Option<Accent> {
    let run = |cmd: &str, args: &[&str]| {
        let out = std::process::Command::new(cmd).args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    if cfg!(target_os = "macos") {
        // Absent key means the default (blue) accent.
        return Some(
            match run("defaults", &["read", "-g", "AppleAccentColor"]).as_deref() {
                Some("0") => Accent::Red,
                Some("1" | "2") => Accent::Orange,
                Some("3") => Accent::Green,
                Some("5") => Accent::Purple,
                Some("6") => Accent::Pink,
                _ => Accent::Blue,
            },
        );
    }
    // shortcut: GNOME 47+ only; other desktops fall back, add the portal setting when asked.
    let v = run(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "accent-color"],
    )?;
    Accent::parse(v.trim_matches('\''))
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_accent(a: Accent) {
    CURRENT.store(
        Accent::ALL.iter().position(|x| *x == a).unwrap_or(0) as u8,
        Ordering::Relaxed,
    );
}

pub fn current_accent() -> Accent {
    Accent::ALL[usize::from(CURRENT.load(Ordering::Relaxed)).min(Accent::ALL.len() - 1)]
}

impl Theme {
    /// Palette with `accent` applied (its text colour stays the theme's `on_accent`).
    pub fn tokens_with(self, accent: Accent) -> crate::DesignTokens {
        let mut t = self.base_tokens();
        if let Some(c) = accent.tone(self.is_dark()) {
            t.accent = c;
        }
        t
    }
}
