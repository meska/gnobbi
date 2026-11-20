//! Tray / menu-bar menu as plain data: no window, no tray crate, no platform code.
//! The icon integration turns `TrayItem`s into native entries and sends `TrayAction`s back
//! over a channel that the app drains (like `drain_navigation_actions`).
use crate::AccountRow;
use matrix_domain::AccountId;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};

pub const MINIMIZE: &str = "Riduci a icona";
pub const SHOW: &str = "Mostra Gnobbi";
/// Fixed label; the native check mark carries the state.
pub const NOTIFICATIONS: &str = "Notifiche";
pub const QUIT: &str = "Chiudi Gnobbi";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    /// Minimize when the window is visible, show it when hidden.
    ToggleWindow,
    /// Primary click on the icon: show, restore and focus, never minimize.
    ShowWindow,
    /// Same switch as Settings: flips `NotificationPolicy::enabled` of the shown account.
    ToggleNotifications,
    Quit,
    /// Shows the account (catalog ID) and brings the window to the front.
    SwitchAccount(AccountId),
    /// macOS menu bar: Edit entry, forwarded to the focused egui text field.
    Edit(Edit),
    /// macOS menu bar: Gnobbi > Impostazioni… (Cmd+,).
    Settings,
}

/// The Edit menu entries of the macOS menu bar (ids `edit:<name>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl Edit {
    pub const ALL: [Edit; 6] = [Edit::Undo, Edit::Redo, Edit::Cut, Edit::Copy, Edit::Paste, Edit::SelectAll];

    pub fn name(self) -> &'static str {
        match self {
            Edit::Undo => "undo",
            Edit::Redo => "redo",
            Edit::Cut => "cut",
            Edit::Copy => "copy",
            Edit::Paste => "paste",
            Edit::SelectAll => "select-all",
        }
    }

    pub fn from_name(name: &str) -> Option<Edit> {
        Edit::ALL.into_iter().find(|e| e.name() == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayItem {
    Entry {
        action: TrayAction,
        label: String,
        /// Spoken name; no separate help text.
        accessible_name: String,
        /// Check mark: only the shown account.
        checked: bool,
        /// Account entries: initials of the drawer row, the fallback of its round avatar.
        avatar_initials: Option<String>,
    },
    Separator,
}

/// Unread over all accounts: the sum of the per-account counts the drawer shows, which already
/// leave out muted rooms (`Sidebar::navigation_entries`). Mentions are not counted.
pub fn total_unread(counts: &HashMap<AccountId, (u64, u64)>) -> u64 {
    counts
        .values()
        .fold(0, |sum, (unread, _)| sum.saturating_add(*unread))
}

fn entry(action: TrayAction, label: &str) -> TrayItem {
    TrayItem::Entry {
        action,
        label: label.into(),
        accessible_name: label.into(),
        checked: false,
        avatar_initials: None,
    }
}

/// The menu for the current state; the account section needs 2+ accounts.
pub fn tray_menu(
    window_visible: bool,
    notifications_enabled: bool,
    accounts: &[AccountRow],
) -> Vec<TrayItem> {
    let mut items = vec![
        entry(
            TrayAction::ToggleWindow,
            if window_visible { MINIMIZE } else { SHOW },
        ),
        {
            let mut item = entry(TrayAction::ToggleNotifications, NOTIFICATIONS);
            if let TrayItem::Entry { checked, .. } = &mut item {
                *checked = notifications_enabled;
            }
            item
        },
        entry(TrayAction::Quit, QUIT),
    ];
    if accounts.len() >= 2 {
        items.push(TrayItem::Separator);
        items.extend(accounts.iter().map(|a| TrayItem::Entry {
            action: TrayAction::SwitchAccount(a.id),
            label: a.name.clone(),
            accessible_name: if a.current {
                format!("{}, account attivo", a.name)
            } else {
                a.name.clone()
            },
            checked: a.current,
            avatar_initials: Some(a.initials.clone()),
        }));
    }
    items
}

pub fn channel_pair() -> (Sender<TrayAction>, Receiver<TrayAction>) {
    channel()
}

/// Up to 8 pending actions per tick, so a burst cannot starve the UI.
pub fn drain_tray_actions(rx: &Receiver<TrayAction>) -> Vec<TrayAction> {
    rx.try_iter().take(8).collect()
}

/// What a window-close request (X button, Super+Q / killactive, Alt+F4) does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    HideToTray,
    Quit,
}

/// Hide only with the setting on and an icon the user can really reach: an app hidden
/// without a tray host would be lost.
pub fn close_action(close_to_tray: bool, tray_reachable: bool) -> CloseAction {
    if close_to_tray && tray_reachable {
        CloseAction::HideToTray
    } else {
        CloseAction::Quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::notifications::NotificationPolicy;

    #[test]
    fn close_hides_only_with_setting_and_reachable_tray() {
        assert_eq!(close_action(true, true), CloseAction::HideToTray);
        assert_eq!(close_action(true, false), CloseAction::Quit);
        assert_eq!(close_action(false, true), CloseAction::Quit);
        assert_eq!(close_action(false, false), CloseAction::Quit);
    }

    fn row(id: u64, name: &str, current: bool) -> AccountRow {
        AccountRow {
            id: AccountId::new(id),
            name: name.into(),
            user_id: format!("@{name}:x"),
            initials: crate::initials(name),
            avatar: None,
            current,
            unread: 0,
            mentions: 0,
        }
    }

    fn labels(menu: &[TrayItem]) -> Vec<&str> {
        menu.iter()
            .map(|i| match i {
                TrayItem::Entry { label, .. } => label.as_str(),
                TrayItem::Separator => "-",
            })
            .collect()
    }

    #[test]
    fn unread_is_summed_over_all_accounts() {
        let counts = HashMap::from([
            (AccountId::new(1), (3, 1)),
            (AccountId::new(2), (4, 0)),
            (AccountId::new(3), (0, 0)),
        ]);
        assert_eq!(total_unread(&counts), 7);
        assert_eq!(total_unread(&HashMap::new()), 0);
    }

    #[test]
    fn one_account_has_no_account_section() {
        let menu = tray_menu(true, true, &[row(1, "Anna", true)]);
        assert_eq!(labels(&menu), [MINIMIZE, NOTIFICATIONS, QUIT],);
        assert!(tray_menu(true, true, &[]).len() == 3);
    }

    #[test]
    fn two_accounts_add_separator_and_names_with_check_on_the_shown_one() {
        let menu = tray_menu(true, true, &[row(1, "Anna", false), row(2, "Bruno", true)]);
        assert_eq!(
            labels(&menu),
            [MINIMIZE, NOTIFICATIONS, QUIT, "-", "Anna", "Bruno"],
        );
        let checks: Vec<_> = menu
            .iter()
            .filter_map(|i| match i {
                TrayItem::Entry {
                    action: TrayAction::SwitchAccount(id),
                    checked,
                    ..
                } => Some((id.as_u64(), *checked)),
                _ => None,
            })
            .collect();
        assert_eq!(checks, [(1, false), (2, true)]);
        let TrayItem::Entry {
            accessible_name,
            avatar_initials,
            ..
        } = &menu[5]
        else {
            panic!()
        };
        assert_eq!(accessible_name, "Bruno, account attivo");
        assert_eq!(avatar_initials.as_deref(), Some("Br"));
    }

    #[test]
    fn window_entry_follows_visibility() {
        assert_eq!(labels(&tray_menu(true, true, &[]))[0], MINIMIZE);
        assert_eq!(labels(&tray_menu(false, true, &[]))[0], SHOW);
    }

    #[test]
    fn notification_entry_reads_the_same_policy_as_settings() {
        let mut policy = NotificationPolicy::new(AccountId::new(1));
        let checked = |on| matches!(tray_menu(true, on, &[])[1], TrayItem::Entry { checked: true, .. });
        assert_eq!(checked(policy.enabled()), policy.enabled());
        // Settings and tray both end in set_enabled on this one policy: no second flag.
        policy.set_enabled(!policy.enabled());
        assert_eq!(checked(policy.enabled()), policy.enabled());
        assert_eq!(labels(&tray_menu(true, policy.enabled(), &[]))[1], NOTIFICATIONS);
    }

    #[test]
    fn actions_travel_over_the_channel_in_order_and_a_burst_is_capped() {
        let (tx, rx) = channel_pair();
        for _ in 0..10 {
            tx.send(TrayAction::Quit).unwrap();
        }
        tx.send(TrayAction::SwitchAccount(AccountId::new(2)))
            .unwrap();
        assert_eq!(drain_tray_actions(&rx).len(), 8);
        assert_eq!(
            drain_tray_actions(&rx).last(),
            Some(&TrayAction::SwitchAccount(AccountId::new(2)))
        );
    }

    #[test]
    fn edit_names_round_trip() {
        for e in Edit::ALL {
            assert_eq!(Edit::from_name(e.name()), Some(e));
        }
        assert_eq!(Edit::from_name("nope"), None);
    }
}
