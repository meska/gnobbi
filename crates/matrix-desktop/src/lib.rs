//! App layer shared by the egui frontend (bin `gnobbi`): state, preferences, projections; no UI toolkit.

pub mod build_info;
pub mod feedback;
pub mod hooks;
pub mod crash_log;
pub mod composer_image;

pub mod i18n;
pub mod new_chat;
pub mod notification_avatar;
pub mod notifications;
pub mod push;
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
pub use signin_form::{FormStatus, SetupPhase, SignInForm, SignInSource, visible_fields};
mod account_catalog;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod account_cli;
mod account_list;
mod app_dirs;
pub mod app_group;
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
    AutoNight, Density, FontChoice, IconChoice, Metrics, Preferences, SCALE_RANGE, TextSize, dynamic_type_scale, fit_zoom, Theme, parse_gsettings_scale, parse_windows_text_scale, is_night_hour, local_hour, set_font,
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
    LinkConfirmation, LinkConfirmationRequest, MessageLink, SafeWebUrl, message_links, open_url, open_with,
};
mod voice;
pub mod audio_player;
pub mod voice_recorder;
pub use voice::{max_note_samples, voice_attachment};
mod live_media;
pub use live_media::LiveMedia;
mod live_operations;
pub use live_operations::{LiveOperations, Wish, overlay as overlay_reactions};
pub mod read_track;
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
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod tray; // tray-icon does not exist on iOS or Android
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

/// Colours and measures come from the generated design tokens (matrix-domain, design/tokens.toml): the one source.
const fn from_design(p: matrix_domain::design::Palette) -> DesignTokens {
    let l = matrix_domain::design::LAYOUT;
    DesignTokens {
        width: l.width, height: l.height, sidebar: l.sidebar, header: l.header, bottom_bar: l.bottom_bar,
        surface: p.surface, text: p.text, secondary: p.secondary, placeholder: p.placeholder, accent: p.accent,
        timeline: p.timeline, outgoing: p.outgoing, selected: p.selected, border: p.border, input: p.input,
        disabled: p.disabled, selection: p.selection, mention_row: p.mention_row, error: p.error,
        on_accent: p.on_accent, avatar_text: p.avatar_text,
    }
}

pub const LIGHT: DesignTokens = from_design(matrix_domain::design::THEMES[0].palette);
/// «Giorno»: bright and neutral, stronger text than «Classico».
pub const DAY: DesignTokens = from_design(matrix_domain::design::THEMES[1].palette);
pub const DARK: DesignTokens = from_design(matrix_domain::design::THEMES[2].palette);
/// «Notte»: near-black surfaces for dark rooms.
pub const NIGHT: DesignTokens = from_design(matrix_domain::design::THEMES[3].palette);
