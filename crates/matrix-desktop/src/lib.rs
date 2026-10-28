//! Original desktop presentation contract; no network or SDK state.

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub mod accessibility;
pub mod notifications;
mod verification;
#[cfg(feature = "desktop")]
pub mod window;
pub use verification::DeviceVerification;
#[cfg(all(feature = "live", unix))]
mod signin_form;
#[cfg(all(feature = "live", unix))]
pub use signin_form::{
    COPY_LABEL, DEFER_LABEL, RECOVER_LABEL, RECOVERY_KEY_LABEL, RECOVERY_NOTE, RECOVERY_PENDING,
    RECOVERY_WARNING, RETRY_LABEL, SAVED_LABEL, SETUP_KEY_LABEL, SETUP_KEY_MASKED, SETUP_KEY_NOTE,
    SETUP_LABEL, SETUP_NOTE, SETUP_PENDING, SETUP_RETRY_LABEL,
};
#[cfg(all(feature = "live", unix))]
pub use signin_form::{FormStatus, SetupPhase, SignInForm, SignInSource, visible_fields};
mod credential_input;
pub use credential_input::{CredentialInput, LoginInput};
mod account_catalog;
mod account_erase;
pub use account_erase::erase_account;
mod account_list;
mod account_state;
pub use account_state::{AccountRoutes, AccountState, Drained};
mod app_dirs;
pub mod desktop_entry;
pub use account_catalog::{AccountCatalog, AccountEntry};
pub use account_list::{AccountRow, account_rows, initials};
pub use app_dirs::{AppDirs, Platform, app_dirs};
pub mod legacy_profile;
mod preferences;
mod rail_layout;
pub use rail_layout::{RailLayout, RailLayouts};
pub use preferences::{Preferences, Theme};
mod composer;
mod connection_options;
pub mod panel_openers;
pub use composer::{Composer, EMOJI_SET, mention_matches, mention_token};
#[cfg(unix)]
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
pub use voice::voice_attachment;
mod live_media;
pub use live_media::LiveMedia;
mod live_delivery;
pub use live_delivery::LiveDelivery;
mod live_operations;
pub use live_operations::LiveOperations;
mod live_history;
pub use live_history::{HistorySession, HistoryStatus};
mod history;
pub use history::{
    DemoEvent, DemoHistory, DemoSendState, bubble_stamp, debug_room_line, opens_message_menu,
};
mod scrollbar;
pub use scrollbar::{ScrollbarGeometry, VirtualScrollExtent};
pub mod row_text;
mod sidebar;
pub use sidebar::{Navigation, NavigationEntry, Sidebar, SidebarRoom, reading_offset};

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
    text: 0x17212b,
    secondary: 0x465b70,
    accent: 0x1264aa,
    timeline: 0xe8f0ed,
    outgoing: 0xd9fdd3,
    selected: 0xe8f3fd,
    border: 0xe3e9ed,
    input: 0xf1f4f6,
    disabled: 0xe8eef2,
    selection: 0xd0e5fa,
    error: 0xb43030,
    on_accent: 0xffffff,
    avatar_text: 0x101922,
};

pub const DARK: DesignTokens = DesignTokens {
    surface: 0x17212b,
    text: 0xe5edf5,
    secondary: 0xb3c5d5,
    accent: 0x80c8ff,
    timeline: 0x101922,
    outgoing: 0x244436,
    selected: 0x263b50,
    border: 0x41586b,
    input: 0x202f3d,
    disabled: 0x263442,
    selection: 0x294563,
    error: 0xffb3b3,
    on_accent: 0x101922,
    ..LIGHT
};

/// «Giorno»: bright and neutral, stronger text than «Classico».
pub const DAY: DesignTokens = DesignTokens {
    text: 0x111418,
    secondary: 0x474f58,
    accent: 0x0b5cad,
    timeline: 0xf4f6f8,
    outgoing: 0xe3f1ff,
    selected: 0xe8f0fa,
    border: 0xd5dbe1,
    input: 0xf1f3f5,
    disabled: 0xe6e9ec,
    selection: 0xcfe3fb,
    error: 0xb00020,
    ..LIGHT
};

/// «Notte»: near-black surfaces for dark rooms.
pub const NIGHT: DesignTokens = DesignTokens {
    surface: 0x0b0e12,
    text: 0xe8edf2,
    secondary: 0xa9b6c3,
    accent: 0x7cc4ff,
    timeline: 0x000000,
    outgoing: 0x16301f,
    selected: 0x16222e,
    border: 0x3a4856,
    input: 0x151a20,
    disabled: 0x1b222a,
    selection: 0x1f3a56,
    error: 0xffb3b3,
    on_accent: 0x000000,
    ..LIGHT
};
