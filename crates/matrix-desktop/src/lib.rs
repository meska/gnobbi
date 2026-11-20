//! App layer shared by the egui frontend (bin `gnobbi`): state, preferences, projections; no UI toolkit.

pub mod build_info;
pub mod i18n;
pub mod new_chat;
pub mod notifications;
pub mod search_list;
pub mod update;

/// The Linux build pins DejaVu; Windows has no DejaVu.
#[cfg(not(windows))]
pub const UI_FONT: &str = "DejaVu Sans";
#[cfg(windows)]
pub const UI_FONT: &str = "Segoe UI";
#[cfg(not(windows))]
pub const SERIF_FONT: &str = "DejaVu Serif";
#[cfg(windows)]
pub const SERIF_FONT: &str = "Georgia";
#[cfg(not(windows))]
pub const MONO_FONT: &str = "DejaVu Sans Mono";
#[cfg(windows)]
pub const MONO_FONT: &str = "Consolas";
mod verification;
mod credential_input;
pub use credential_input::{CredentialInput, LoginInput, line_range, word_range};
pub mod app_icon;
pub mod directory_rooms;
mod link_requests;
pub use link_requests::{LinkMenu, LinkSource, MessageLinkRequests, MessageLinkTarget, link_at};
pub mod inactive;
pub mod key_export;
pub mod app_id;
pub use verification::{DeviceVerification, PopupBody, PopupButton, PopupView, device_detail, device_label, device_title};
mod signin_form;
pub use signin_form::{
    COPY_LABEL, DEFER_LABEL, RECOVER_LABEL, RECOVERY_KEY_LABEL, RECOVERY_NOTE, RECOVERY_PENDING,
    RECOVERY_WARNING, RETRY_LABEL, SAVED_LABEL, SETUP_KEY_LABEL, SETUP_KEY_MASKED, SETUP_KEY_NOTE,
    SETUP_LABEL, SETUP_NOTE, SETUP_PENDING, SETUP_RETRY_LABEL,
};
pub use signin_form::{FormStatus, SetupPhase, SignInForm, SignInSource, visible_fields};
mod account_catalog;
mod account_list;
mod app_dirs;
pub mod desktop_entry;
// Linux only: the uid comes from /proc/self, and on macOS nothing calls it (the OS opens links itself).
#[cfg(target_os = "linux")]
pub mod single_instance;
pub mod windows_link;

#[cfg(windows)]
pub use windows_link as single_instance;
pub use account_catalog::{AccountCatalog, AccountEntry};
pub use account_list::{AccountRow, account_rows, initials};
pub use app_dirs::{AppDirs, Platform, app_dirs};
mod accent;
pub use accent::{Accent, current_accent, set_accent, system_accent};
mod preferences;
pub mod quick_search;
pub mod add_server;
pub mod directory_servers;
mod rail_layout;
mod room_status;
pub use room_status::{RoomStatusChoices, StatusChoice};
pub use preferences::{
    AutoNight, Density, FontChoice, IconChoice, Metrics, Preferences, SCALE_RANGE, TextSize, Theme, parse_gsettings_scale, parse_windows_text_scale, is_night_hour, local_hour, set_font,
    ui_font,
};
pub use rail_layout::{RailLayout, RailLayouts};
mod composer;
mod connection_options;
pub use composer::{
    Composer, EMOJI_SET, mention_matches, mention_token, next_boundary, prev_boundary,
};
pub use connection_options::create_private_dir;
pub use connection_options::{
    RestoreOptions, SignInOptions, bundle_launch_args, default_signin_args, linux_launch_args,
    take_matrix_link,
};
mod links;
pub mod mention_gaps;
pub use links::{
    LinkConfirmation, LinkConfirmationRequest, MessageLink, SafeWebUrl, message_links,
};
mod voice;
pub mod audio_player;
pub mod voice_recorder;
pub use voice::{max_note_samples, voice_attachment};
mod live_media;
pub use live_media::LiveMedia;
mod live_operations;
pub use live_operations::LiveOperations;
mod live_history;
pub use live_history::{HistorySession, HistoryStatus, SETTLE};
mod history;
pub use history::{
    DemoEvent, DemoHistory, DemoSendState, StampMark, bubble_stamp, stamp_mark, debug_room_line, opens_message_menu,
};
pub mod big_emoji;
pub mod row_text;
pub mod time_fmt;
mod sidebar;
pub mod tray_menu;
#[cfg(not(target_os = "ios"))]
pub mod tray; // tray-icon does not exist on iOS
pub use sidebar::{InviteState, Navigation, group_label, NavigationEntry, Sidebar, SidebarRoom, reading_offset};

#[derive(Clone, Copy, Debug)]
pub struct DesignTokens {
    pub width: f32,
    pub height: f32,
    pub sidebar: f32,
    pub header: f32,
    pub bottom_bar: f32,
    pub surface: u32,
    pub text: u32,
    pub secondary: u32,
    pub placeholder: u32,
    pub accent: u32,
    pub timeline: u32,
    pub outgoing: u32,
    pub selected: u32,
    pub border: u32,
    pub input: u32,
    pub disabled: u32,
    pub selection: u32,
    pub mention_row: u32,
    pub error: u32,
    pub on_accent: u32,
    pub avatar_text: u32,
}


/// Avatar fallback colour: identity, not theme.
pub const AVATAR_FALLBACK: u32 = 0x53936b;



pub const DEMO_NOTICE: &str = "DEMO OFFLINE · nessun account connesso";

/// Scrim behind dialogs (RGBA): the same in every theme.
pub const SCRIM: u32 = 0x0000_0066;

pub struct DemoRoom {
    pub name: &'static str,
    pub initials: &'static str,
    pub preview: &'static str,
    pub time: &'static str,
    pub color: u32,
}

pub const ROOMS: [DemoRoom; 6] = [
    DemoRoom {
        name: "Laboratorio UI",
        initials: "LU",
        preview: "Una shell originale, senza rete",
        time: "10:46",
        color: 0x609bc2,
    },
    DemoRoom {
        name: "Design desktop",
        initials: "DD",
        preview: "Spazi, colori e leggibilità",
        time: "09:32",
        color: 0xa58ac4,
    },
    DemoRoom {
        name: "Sviluppo Rust",
        initials: "SR",
        preview: "Core separato dalla presentazione",
        time: "Ieri",
        color: 0x6aa68e,
    },
    DemoRoom {
        name: "Idee e appunti",
        initials: "IA",
        preview: "Piccoli dettagli, grandi differenze",
        time: "Ieri",
        color: 0xc99766,
    },
    DemoRoom {
        name: "Percorsi locali",
        initials: "PL",
        preview: "Solo fixture deterministiche",
        time: "Lun",
        color: 0x8a99bc,
    },
    DemoRoom {
        name: "Accessibilità",
        initials: "AC",
        preview: "Un passo alla volta",
        time: "Lun",
        color: 0xb67d8e,
    },
];

pub const LIGHT: DesignTokens = DesignTokens {
    width: 1120.,
    height: 760.,
    sidebar: 320.,
    header: 64.,
    bottom_bar: 56.,
    surface: 0xffffff,
    text: 0x17231b,
    secondary: 0x46604e,
    placeholder: matrix_domain::design::THEMES[0].palette.placeholder,
    accent: 0x1264aa,
    timeline: 0xe9f0eb,
    outgoing: 0xd9fdd3,
    selected: 0xe9f5ec,
    border: 0xe4eae6,
    input: 0xf1f4f2,
    disabled: 0xe9efeb,
    selection: 0xd0ead8,
    mention_row: 0xe2d5d1,
    error: 0xb43030,
    on_accent: 0xffffff,
    avatar_text: 0x101b14,
};

pub const DARK: DesignTokens = DesignTokens {
    surface: 0x17231b,
    text: 0xe5efe8,
    secondary: 0xb4c9ba,
    placeholder: matrix_domain::design::THEMES[2].palette.placeholder,
    accent: 0x80c8ff,
    timeline: 0x101b14,
    outgoing: 0x244436,
    selected: 0x25402f,
    border: 0x425c4b,
    input: 0x203226,
    disabled: 0x26372c,
    selection: 0x254d34,
    mention_row: 0x3b3631,
    error: 0xffb3b3,
    on_accent: 0x101922,
    ..LIGHT
};

/// «Giorno»: bright and neutral, stronger text than «Classico».
pub const DAY: DesignTokens = DesignTokens {
    text: 0x111512,
    secondary: 0x47514a,
    placeholder: matrix_domain::design::THEMES[1].palette.placeholder,
    accent: 0x0b5cad,
    timeline: 0xf4f6f5,
    outgoing: 0xd2f6dd,
    selected: 0xe7f3ea,
    border: 0xd5dcd7,
    input: 0xf1f3f2,
    disabled: 0xe6eae7,
    selection: 0xcde9d6,
    mention_row: 0xead4d7,
    error: 0xb00020,
    ..LIGHT
};

/// «Notte»: near-black surfaces for dark rooms.
pub const NIGHT: DesignTokens = DesignTokens {
    surface: 0x0b0f0c,
    text: 0xe8eeea,
    secondary: 0xa9b9ae,
    placeholder: matrix_domain::design::THEMES[3].palette.placeholder,
    accent: 0x7cc4ff,
    timeline: 0x000000,
    outgoing: 0x16301f,
    selected: 0x16251b,
    border: 0x3a4b3f,
    input: 0x141c17,
    disabled: 0x1a241d,
    selection: 0x1b412a,
    mention_row: 0x2e2020,
    error: 0xffb3b3,
    on_accent: 0x000000,
    ..LIGHT
};
