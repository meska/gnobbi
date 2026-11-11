//! What a `matrix:` link does once it is validated: which account it goes to and which existing screen
//! it opens. Pure, so every case is a plain test; `link_view.rs` only carries the plan out.
//! A link never acts by itself: it opens a room, shows an invitation, opens «Unisciti» or a profile.
use matrix_domain::{AccountId, MatrixLink, RoomScope};

/// How the accounts are shown. Only `Separate` exists today; the other two are here so the rule of
/// the MULTI_ACCOUNT_MODES notes §8 is one tested function when those modes are built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A: one account shown, the others parked. `shown` is that account.
    Separate,
    /// B: one window per account. `shown` is the account of the window in front.
    PerWindow,
    /// C: merged list. `shown` is the account of the chat selected now.
    Merged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Open(AccountId),
    Join(AccountId),
    /// Merged mode, several accounts inside and none selected: a dialog lists them by full user ID.
    Ask(Vec<AccountId>),
    NoAccount,
}

/// `inside`: accounts already in the room (for a user link, every usable account).
pub fn pick_account(link: &MatrixLink, inside: &[AccountId], mode: Mode, shown: Option<AccountId>) -> Choice {
    match inside {
        [] => match (link, shown) {
            (MatrixLink::Room { .. }, Some(shown)) => Choice::Join(shown),
            _ => Choice::NoAccount,
        },
        [one] => Choice::Open(*one),
        many => match (shown.filter(|s| many.contains(s)), mode) {
            (Some(shown), _) => Choice::Open(shown),
            (None, Mode::Merged) => Choice::Ask(many.to_vec()),
            (None, _) => Choice::Open(many[0]),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    Joined(RoomScope),
    Invited(RoomScope),
    Absent,
}

/// A usable account and what it knows about the room a link names (`Absent` for user links).
pub struct Candidate {
    pub id: AccountId,
    pub presence: Presence,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Room { account: AccountId, scope: RoomScope },
    Invite { account: AccountId, scope: RoomScope },
    Join { account: AccountId, target: String, via: Vec<String> },
    Profile { account: AccountId, user_id: String },
    Ask(Vec<AccountId>),
    Notice(&'static str),
}

const NO_ACCOUNT: &str = "Account non disponibile";

pub fn plan(link: &MatrixLink, accounts: &[Candidate], mode: Mode, shown: Option<AccountId>) -> Plan {
    let joined = |c: &Candidate| matches!(c.presence, Presence::Joined(_));
    match link {
        MatrixLink::User { user_id } => {
            let all: Vec<_> = accounts.iter().map(|c| c.id).collect();
            match pick_account(link, &all, mode, shown) {
                Choice::Open(account) => Plan::Profile { account, user_id: user_id.clone() },
                Choice::Ask(ids) => Plan::Ask(ids),
                _ => Plan::Notice(NO_ACCOUNT),
            }
        }
        // The event, if any, is left to the card that scrolls to it: here the room only opens.
        MatrixLink::Room { target, via, .. } => {
            let inside: Vec<_> = accounts.iter().filter(|c| joined(c)).map(|c| c.id).collect();
            match pick_account(link, &inside, mode, shown) {
                Choice::Open(id) => accounts
                    .iter()
                    .find_map(|c| match &c.presence {
                        Presence::Joined(scope) if c.id == id => Some(Plan::Room { account: id, scope: scope.clone() }),
                        _ => None,
                    })
                    .unwrap_or(Plan::Notice(NO_ACCOUNT)),
                Choice::Ask(ids) => Plan::Ask(ids),
                // In no account yet: a pending invitation is answered where it is, else «Unisciti».
                Choice::Join(account) => accounts
                    .iter()
                    .find_map(|c| match &c.presence {
                        Presence::Invited(scope) => Some(Plan::Invite { account: c.id, scope: scope.clone() }),
                        _ => None,
                    })
                    .unwrap_or(Plan::Join { account, target: target.clone(), via: via.clone() }),
                Choice::NoAccount => Plan::Notice(NO_ACCOUNT),
            }
        }
    }
}

/// The one link waiting for the window to be ready: the last one wins, never written anywhere.
#[derive(Default)]
pub struct LinkQueue(Option<MatrixLink>);

impl LinkQueue {
    pub fn push(&mut self, link: MatrixLink) {
        self.0 = Some(link);
    }

    pub fn extend(&mut self, link: Option<MatrixLink>) {
        self.0 = link.or(self.0.take());
    }

    pub fn waiting(&self) -> bool {
        self.0.is_some()
    }

    /// The waiting link, once the accounts are ready and no modal dialog is open.
    pub fn take(&mut self, ready: bool, modal: bool) -> Option<MatrixLink> {
        if ready && !modal { self.0.take() } else { None }
    }
}

/// Only the kind of a link may reach a log.
pub fn kind(link: &MatrixLink) -> &'static str {
    match link {
        MatrixLink::Room { .. } => "room",
        MatrixLink::User { .. } => "user",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::RoomId;

    fn id(n: u64) -> AccountId {
        AccountId::new(n)
    }
    fn scope(n: u64) -> RoomScope {
        RoomScope { account_id: id(n), room_id: RoomId::new("!r:s.org") }
    }
    fn room() -> MatrixLink {
        MatrixLink::Room { target: "!r:s.org".into(), via: vec!["a.org".into()], event: None }
    }
    fn user() -> MatrixLink {
        MatrixLink::User { user_id: "@u:s.org".into() }
    }
    fn cand(n: u64, presence: Presence) -> Candidate {
        Candidate { id: id(n), presence }
    }

    #[test]
    fn one_account_inside_is_the_one_in_every_mode() {
        for mode in [Mode::Separate, Mode::PerWindow, Mode::Merged] {
            assert_eq!(pick_account(&room(), &[id(2)], mode, Some(id(1))), Choice::Open(id(2)));
        }
    }

    #[test]
    fn several_inside_follow_the_mode() {
        let inside = [id(1), id(2)];
        for mode in [Mode::Separate, Mode::PerWindow, Mode::Merged] {
            // The shown account (A), the window in front (B), the selected chat (C).
            assert_eq!(pick_account(&room(), &inside, mode, Some(id(2))), Choice::Open(id(2)));
        }
        // Shown one is not inside: A and B take the first inside, C asks (by full user ID).
        assert_eq!(pick_account(&room(), &inside, Mode::Separate, Some(id(3))), Choice::Open(id(1)));
        assert_eq!(pick_account(&room(), &inside, Mode::PerWindow, None), Choice::Open(id(1)));
        assert_eq!(pick_account(&room(), &inside, Mode::Merged, Some(id(3))), Choice::Ask(vec![id(1), id(2)]));
    }

    #[test]
    fn nobody_inside_joins_with_the_shown_account() {
        for mode in [Mode::Separate, Mode::PerWindow, Mode::Merged] {
            assert_eq!(pick_account(&room(), &[], mode, Some(id(1))), Choice::Join(id(1)));
            assert_eq!(pick_account(&room(), &[], mode, None), Choice::NoAccount);
        }
        assert_eq!(pick_account(&user(), &[], Mode::Separate, Some(id(1))), Choice::NoAccount);
    }

    #[test]
    fn joined_room_opens() {
        let accounts = [cand(1, Presence::Absent), cand(2, Presence::Joined(scope(2)))];
        assert_eq!(plan(&room(), &accounts, Mode::Separate, Some(id(1))), Plan::Room { account: id(2), scope: scope(2) });
    }

    #[test]
    fn two_accounts_inside_prefer_the_shown_one() {
        let accounts = [cand(1, Presence::Joined(scope(1))), cand(2, Presence::Joined(scope(2)))];
        assert_eq!(plan(&room(), &accounts, Mode::Separate, Some(id(2))), Plan::Room { account: id(2), scope: scope(2) });
        assert_eq!(plan(&room(), &accounts, Mode::Merged, None), Plan::Ask(vec![id(1), id(2)]));
    }

    #[test]
    fn not_joined_proposes_join_and_never_joins() {
        let accounts = [cand(1, Presence::Absent), cand(2, Presence::Absent)];
        assert_eq!(
            plan(&room(), &accounts, Mode::Separate, Some(id(1))),
            Plan::Join { account: id(1), target: "!r:s.org".into(), via: vec!["a.org".into()] }
        );
    }

    #[test]
    fn pending_invitation_shows_its_row() {
        let accounts = [cand(1, Presence::Absent), cand(2, Presence::Invited(scope(2)))];
        assert_eq!(plan(&room(), &accounts, Mode::Separate, Some(id(1))), Plan::Invite { account: id(2), scope: scope(2) });
    }

    #[test]
    fn event_link_opens_the_room_only() {
        let link = MatrixLink::Room { target: "!r:s.org".into(), via: vec![], event: Some("$e".into()) };
        let accounts = [cand(1, Presence::Joined(scope(1)))];
        assert_eq!(plan(&link, &accounts, Mode::Separate, Some(id(1))), Plan::Room { account: id(1), scope: scope(1) });
    }

    #[test]
    fn user_link_goes_to_the_shown_account_or_the_only_one() {
        let two = [cand(1, Presence::Absent), cand(2, Presence::Absent)];
        assert_eq!(plan(&user(), &two, Mode::Separate, Some(id(2))), Plan::Profile { account: id(2), user_id: "@u:s.org".into() });
        assert_eq!(plan(&user(), &two[..1], Mode::Separate, None), Plan::Profile { account: id(1), user_id: "@u:s.org".into() });
        assert_eq!(plan(&user(), &[], Mode::Separate, Some(id(1))), Plan::Notice("Account non disponibile"));
    }

    #[test]
    fn last_link_wins_and_waits_for_ready_and_no_dialog() {
        let mut q = LinkQueue::default();
        q.push(room());
        q.push(user());
        assert_eq!(q.take(false, false), None, "accounts not ready");
        assert_eq!(q.take(true, true), None, "a dialog is open");
        assert!(q.waiting());
        assert_eq!(q.take(true, false), Some(user()));
        assert_eq!(q.take(true, false), None, "disposed once");
    }
}
