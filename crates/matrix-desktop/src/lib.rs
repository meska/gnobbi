//! Original desktop presentation contract; no network or SDK state.

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub mod accessibility;
pub mod new_chat;
pub mod notifications;
pub mod search_list;
/// The Linux build pins DejaVu; Windows has no DejaVu and gpui panics on a missing family.
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
#[cfg(feature = "desktop")]
pub mod window;
pub use verification::DeviceVerification;
#[cfg(feature = "live")]
mod signin_form;
#[cfg(feature = "live")]
pub use signin_form::{
    COPY_LABEL, DEFER_LABEL, RECOVER_LABEL, RECOVERY_KEY_LABEL, RECOVERY_NOTE, RECOVERY_PENDING,
    RECOVERY_WARNING, RETRY_LABEL, SAVED_LABEL, SETUP_KEY_LABEL, SETUP_KEY_MASKED, SETUP_KEY_NOTE,
    SETUP_LABEL, SETUP_NOTE, SETUP_PENDING, SETUP_RETRY_LABEL,
};
#[cfg(feature = "live")]
pub use signin_form::{FormStatus, SetupPhase, SignInForm, SignInSource, visible_fields};
mod credential_input;
pub use credential_input::{CredentialInput, LoginInput};
mod account_catalog;
mod account_erase;
pub mod app_icon;
pub use account_erase::erase_account;
mod account_list;
mod account_state;
pub use account_state::{AccountRoutes, AccountState, Drained};
mod app_dirs;
pub mod desktop_entry;
pub use account_catalog::{AccountCatalog, AccountEntry};
pub use account_list::{AccountRow, account_rows, initials};
pub use app_dirs::{AppDirs, Platform, app_dirs};
mod accent;
pub mod legacy_profile;
pub use accent::{Accent, current_accent, set_accent, system_accent};
mod preferences;
mod rail_layout;
mod room_prefs;
pub use preferences::{
    AutoNight, FontChoice, IconChoice, Preferences, Theme, is_night_hour, local_hour, set_font,
    ui_font,
};
pub use rail_layout::{RailLayout, RailLayouts};
pub use room_prefs::RoomPrefs;
mod composer;
mod connection_options;
pub mod panel_openers;
pub use composer::{Composer, EMOJI_SET, mention_matches, mention_token};
pub use connection_options::create_private_dir;
pub use connection_options::{
    RestoreOptions, SignInOptions, bundle_launch_args, default_signin_args, linux_launch_args,
};
mod drafts;
pub use drafts::RoomDrafts;
mod selection;
pub use selection::TextSelection;
mod offline_delivery;
pub use offline_delivery::{OfflineDelivery, OfflineDeliveryTarget};
mod event_id_copy;
pub use event_id_copy::{EventIdCopies, EventIdCopyTarget};
mod link_requests;
pub use link_requests::{LinkMenu, MessageLinkRequests, MessageLinkTarget, link_at};
mod links;
pub use links::{
    LinkConfirmation, LinkConfirmationRequest, MessageLink, SafeWebUrl, message_links,
};
mod voice;
pub use voice::{max_note_samples, voice_attachment};
mod attachment_save;
mod live_media;
pub use attachment_save::{
    SaveState, Saves, failure_label as save_failure_label, offers_save, over_cap as save_over_cap,
    retryable as save_retryable, start_dir as save_start_dir,
};
pub use live_media::LiveMedia;
mod live_delivery;
pub use live_delivery::LiveDelivery;
mod live_operations;
pub use live_operations::LiveOperations;
mod live_history;
pub use live_history::{HistorySession, HistoryStatus, SETTLE};
mod history;
pub use history::{
    DemoEvent, DemoHistory, DemoSendState, bubble_stamp, debug_room_line, opens_message_menu,
};
mod scrollbar;
pub use scrollbar::{ScrollbarGeometry, VirtualScrollExtent};
pub mod row_text;
mod sidebar;
pub use sidebar::{
    InviteState, Navigation, NavigationEntry, Sidebar, SidebarRoom, group_label, reading_offset,
};

#[derive(Clone, Copy, Debug)]
pub struct DesignTokens {
    pub width: f32,
    pub height: f32,
    pub sidebar: f32,
    pub header: f32,
    pub composer: f32,
    pub surface: u32,
    pub text: u32,
    pub secondary: u32,
    pub accent: u32,
    pub timeline: u32,
    pub outgoing: u32,
    pub selected: u32,
    pub border: u32,
    pub input: u32,
    pub disabled: u32,
    pub selection: u32,
    pub error: u32,
    pub on_accent: u32,
    pub avatar_text: u32,
}

pub const DEMO_NOTICE: &str = "DEMO OFFLINE · nessun account connesso";

/// Scrim behind dialogs (RGBA): the same in every theme.
pub const SCRIM: u32 = 0x0000_0066;
/// Avatar fallback colour: identity, not theme.
pub const AVATAR_FALLBACK: u32 = 0x53936b;

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

pub struct DemoMessage {
    pub text: &'static str,
    pub time: &'static str,
    pub outgoing: bool,
}

pub const MESSAGES: [DemoMessage; 5] = [
    DemoMessage {
        text: "Questa è la nuova shell desktop. Tutti i contenuti sono fixture locali.",
        time: "10:42",
        outgoing: false,
    },
    DemoMessage {
        text: "Due colonne, una lista compatta e spazio per la conversazione.",
        time: "10:43",
        outgoing: true,
    },
    DemoMessage {
        text: "Il core rimane indipendente dalla UI.\nNessun homeserver, login o invio reale in questa demo.",
        time: "10:44",
        outgoing: false,
    },
    DemoMessage {
        text: "Accenti e Unicode: caffè, città, 日本語.\nAnche i messaggi su più righe devono restare leggibili.",
        time: "10:45",
        outgoing: true,
    },
    DemoMessage {
        text: "Il composer è disabilitato: nessun messaggio può essere inviato in questa shell.",
        time: "10:46",
        outgoing: false,
    },
];

pub const LIGHT: DesignTokens = DesignTokens {
    width: 1120.,
    height: 760.,
    sidebar: 320.,
    header: 64.,
    composer: 72.,
    surface: 0xffffff,
    text: 0x17231b,
    secondary: 0x46604e,
    accent: 0x1264aa,
    timeline: 0xe9f0eb,
    outgoing: 0xd9fdd3,
    selected: 0xe9f5ec,
    border: 0xe4eae6,
    input: 0xf1f4f2,
    disabled: 0xe9efeb,
    selection: 0xd0ead8,
    error: 0xb43030,
    on_accent: 0xffffff,
    avatar_text: 0x101b14,
};

pub const DARK: DesignTokens = DesignTokens {
    surface: 0x17231b,
    text: 0xe5efe8,
    secondary: 0xb4c9ba,
    accent: 0x80c8ff,
    timeline: 0x101b14,
    outgoing: 0x244436,
    selected: 0x25402f,
    border: 0x425c4b,
    input: 0x203226,
    disabled: 0x26372c,
    selection: 0x254d34,
    error: 0xffb3b3,
    on_accent: 0x101922,
    ..LIGHT
};

/// «Giorno»: bright and neutral, stronger text than «Classico».
pub const DAY: DesignTokens = DesignTokens {
    text: 0x111512,
    secondary: 0x47514a,
    accent: 0x0b5cad,
    timeline: 0xf4f6f5,
    outgoing: 0xd2f6dd,
    selected: 0xe7f3ea,
    border: 0xd5dcd7,
    input: 0xf1f3f2,
    disabled: 0xe6eae7,
    selection: 0xcde9d6,
    error: 0xb00020,
    ..LIGHT
};

/// «Notte»: near-black surfaces for dark rooms.
pub const NIGHT: DesignTokens = DesignTokens {
    surface: 0x0b0f0c,
    text: 0xe8eeea,
    secondary: 0xa9b9ae,
    accent: 0x7cc4ff,
    timeline: 0x000000,
    outgoing: 0x16301f,
    selected: 0x16251b,
    border: 0x3a4b3f,
    input: 0x141c17,
    disabled: 0x1a241d,
    selection: 0x1b412a,
    error: 0xffb3b3,
    on_accent: 0x000000,
    ..LIGHT
};
