use super::*;
use matrix_engine::{RoomInfo, RoomSummary};

#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);
impl SyncListener for Recorder {
    fn on_status(&self, state: SyncState) {
        self.0.lock().unwrap().push(format!("status {state:?}"));
    }
    fn on_rooms(&self, rooms: Vec<RoomItem>) {
        let ids: Vec<_> = rooms.iter().map(|r| r.room_id.as_str()).collect();
        self.0
            .lock()
            .unwrap()
            .push(format!("rooms {}", ids.join(",")));
    }
    fn on_send_result(&self, room_id: String, _: String, error: Option<MobileError>) {
        self.0
            .lock()
            .unwrap()
            .push(format!("sent {room_id} {error:?}"));
    }
}
impl Recorder {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

struct NoSecrets;
impl SecretStore for NoSecrets {
    fn create(&self, _: u64) -> Result<String, MobileError> {
        Err(MobileError::SecretStore)
    }
    fn read(&self, _: u64) -> Result<String, MobileError> {
        Err(MobileError::SecretStore)
    }
}

fn scope(room: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(1),
        room_id: RoomId::new(room),
    }
}
fn upsert(room: &str, name: &str, unread: u64) -> RoomListEvent {
    RoomListEvent::Diff(RoomListDiff::Upsert(RoomSummary {
        scope: scope(room),
        name: name.into(),
        unread,
        info: RoomInfo::default(),
    }))
}

#[test]
fn rooms_follow_the_engine_order_and_report_once_per_burst() {
    let (mut view, rec) = (RoomsView::default(), Recorder::default());
    view.apply(upsert("!b", "B", 0));
    view.apply(upsert("!a", "A", 2));
    view.apply(upsert("!c", "C", 0));
    view.apply(RoomListEvent::Order(vec![scope("!b"), scope("!a")]));
    view.flush(&rec);
    // Listed rooms in engine order, the not-yet-ordered one after them.
    assert_eq!(rec.take(), ["rooms !b,!a,!c"]);
    view.flush(&rec);
    assert!(rec.take().is_empty(), "nothing changed, nothing to say");
    view.apply(upsert("!a", "A", 2));
    view.flush(&rec);
    assert!(rec.take().is_empty(), "identical upsert is not a change");
    view.apply(RoomListEvent::Diff(RoomListDiff::Remove(scope("!b"))));
    view.flush(&rec);
    assert_eq!(rec.take(), ["rooms !a,!c"]);
}

#[test]
fn status_is_announced_only_when_it_changes_and_a_closed_engine_means_failure() {
    let (mut view, rec) = (RoomsView::default(), Recorder::default());
    view.apply(RoomListEvent::Status(RoomSyncStatus::Connecting));
    view.flush(&rec);
    view.apply(RoomListEvent::Status(RoomSyncStatus::Connecting));
    view.flush(&rec);
    view.closed();
    view.flush(&rec);
    assert_eq!(rec.take(), ["status Connecting", "status SyncFailed"]);
    let mut failed = RoomsView::default();
    failed.apply(RoomListEvent::Status(RoomSyncStatus::AuthenticationFailed));
    failed.closed();
    failed.flush(&rec);
    assert_eq!(rec.take(), ["status AuthenticationFailed"]);
}

#[test]
fn engine_errors_map_to_fixed_categories() {
    use matrix_engine::auth::AuthError as A;
    assert_eq!(
        MobileError::from(SignInError::Vault(VaultError::Locked)),
        MobileError::SecretStore
    );
    assert_eq!(
        MobileError::from(SignInError::Authentication(A::CredentialsRejected)),
        MobileError::Rejected
    );
    assert_eq!(
        MobileError::from(SignInError::Authentication(A::Transport)),
        MobileError::Network
    );
    assert_eq!(
        MobileError::from(SignInError::NoSlidingSync),
        MobileError::Unsupported
    );
    assert_eq!(
        MobileError::from(SendError::Encryption),
        MobileError::Encryption
    );
}

#[test]
fn foreign_secret_failures_stay_vault_errors() {
    let source = ForeignSource(Arc::new(NoSecrets));
    assert_eq!(
        source.secret(Err(MobileError::SecretStore)).err(),
        Some(VaultError::Unavailable)
    );
    assert_eq!(
        source.secret(Ok(String::new())).err(),
        Some(VaultError::InvalidItem)
    );
    assert!(source.secret(Ok("0123456789abcdef".into())).is_ok());
}

fn unique_dir() -> String {
    let name = format!("mm-{}-{:?}", std::process::id(), std::time::Instant::now());
    let name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect();
    std::env::temp_dir().join(name).to_str().unwrap().to_owned()
}

#[tokio::test(flavor = "current_thread")]
async fn restore_without_a_signed_in_account_is_no_session() {
    let dir = unique_dir();
    let outcome = restore_session(
        dir.clone(),
        Arc::new(NoSecrets),
        Arc::new(Recorder::default()),
        None,
    )
    .await;
    assert_eq!(outcome.err(), Some(MobileError::NoSession));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "current_thread")]
async fn sign_in_rejects_bad_input_before_touching_the_network() {
    let (secrets, listener) = (Arc::new(NoSecrets), Arc::new(Recorder::default()));
    let go = |dir: String, hs: &str, user: &str, pw: &str| {
        sign_in(
            dir,
            hs.into(),
            user.into(),
            pw.into(),
            secrets.clone(),
            listener.clone(),
            None,
        )
    };
    assert_eq!(
        go(unique_dir(), "https://x.invalid", "@a:x.invalid", "")
            .await
            .err(),
        Some(MobileError::InvalidInput)
    );
    assert_eq!(
        go(unique_dir(), "http://x.invalid", "@a:x.invalid", "pw")
            .await
            .err(),
        Some(MobileError::InvalidHomeserver)
    );
    assert_eq!(
        go(unique_dir(), "https://x.invalid", "alice", "pw")
            .await
            .err(),
        Some(MobileError::InvalidUserId)
    );
}
