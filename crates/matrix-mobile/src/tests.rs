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
    fn delete(&self, _: u64) -> Result<(), MobileError> {
        Err(MobileError::SecretStore)
    }
}

/// Remembers which accounts had their secret deleted.
#[derive(Default)]
struct Deletes(Mutex<Vec<u64>>);
impl SecretStore for Deletes {
    fn create(&self, _: u64) -> Result<String, MobileError> {
        Err(MobileError::SecretStore)
    }
    fn read(&self, _: u64) -> Result<String, MobileError> {
        Err(MobileError::SecretStore)
    }
    fn delete(&self, account: u64) -> Result<(), MobileError> {
        self.0.lock().unwrap().push(account);
        Ok(())
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
    // Canonical: the engine refuses a path through a symlink (macOS /var -> /private/var).
    let base = std::env::temp_dir().canonicalize().unwrap();
    base.join(name).to_str().unwrap().to_owned()
}

#[tokio::test(flavor = "current_thread")]
async fn restore_without_a_signed_in_account_is_no_session() {
    let dir = unique_dir();
    let outcome = restore_session(
        dir.clone(),
        1,
        Some(Arc::new(NoSecrets)),
        Arc::new(Recorder::default()),
        None,
    )
    .await;
    assert_eq!(outcome.err(), Some(MobileError::NoSession));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "current_thread")]
async fn sign_in_rejects_bad_input_before_touching_the_network() {
    let secrets: Option<Arc<dyn SecretStore>> = Some(Arc::new(NoSecrets));
    let listener = Arc::new(Recorder::default());
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

/// A directory the way a finished sign-in leaves it (selector only; the store is not needed here).
fn fake_saved(data_dir: &str, account: u64) {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::io::Write;
    let root = profile_root(Path::new(data_dir), account);
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&root).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join("active-account"))
        .unwrap();
    file.write_all(account.to_string().as_bytes()).unwrap();
}

#[test]
fn saved_accounts_are_listed_in_order_and_unfinished_ones_are_not() {
    let dir = unique_dir();
    assert_eq!(list_saved_accounts(dir.clone()), Ok(vec![]));
    ensure_dirs(Path::new(&dir)).unwrap();
    fake_saved(&dir, 3);
    fake_saved(&dir, 1);
    // Refused sign-in leftovers, a stray file and ids out of range or not canonical.
    for name in ["2", "0", "129", "01", "x"] {
        std::fs::create_dir_all(Path::new(&dir).join("accounts").join(name)).unwrap();
    }
    assert_eq!(list_saved_accounts(dir.clone()), Ok(vec![1, 3]));
    assert_eq!(free_id(Path::new(&dir)), Ok(4), "2 is taken by a leftover directory");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn accounts_never_share_a_store_directory() {
    let dir = Path::new("/d");
    assert_ne!(profile_root(dir, 1), profile_root(dir, 2));
    assert!(!profile_root(dir, 1).starts_with(profile_root(dir, 2)));
}

#[tokio::test(flavor = "current_thread")]
async fn restoring_an_account_that_is_not_saved_is_no_session_and_creates_nothing() {
    let dir = unique_dir();
    ensure_dirs(Path::new(&dir)).unwrap();
    fake_saved(&dir, 1);
    let outcome = restore_session(dir.clone(), 2, Some(Arc::new(NoSecrets)), Arc::new(Recorder::default()), None).await;
    assert_eq!(outcome.err(), Some(MobileError::NoSession));
    assert!(!profile_root(Path::new(&dir), 2).exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn removing_a_local_account_deletes_only_its_secret_and_directory() {
    let dir = unique_dir();
    ensure_dirs(Path::new(&dir)).unwrap();
    fake_saved(&dir, 1);
    fake_saved(&dir, 2);
    let secrets = Arc::new(Deletes::default());
    assert_eq!(remove_local_account(dir.clone(), 2, Some(secrets.clone())), Ok(()));
    assert_eq!(*secrets.0.lock().unwrap(), [2], "account 1 keeps its secret");
    assert_eq!(list_saved_accounts(dir.clone()), Ok(vec![1]));
    assert_eq!(
        remove_local_account(dir.clone(), 2, Some(secrets)),
        Err(MobileError::NoSession)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_open_account_cannot_be_removed_locally() {
    let dir = unique_dir();
    ensure_dirs(Path::new(&dir)).unwrap();
    fake_saved(&dir, 1);
    let open = SignInProfile::open_as(profile_root(Path::new(&dir), 1), AccountId::new(1)).unwrap();
    let secrets = Arc::new(Deletes::default());
    assert_eq!(remove_local_account(dir.clone(), 1, Some(secrets.clone())), Err(MobileError::Busy));
    assert!(secrets.0.lock().unwrap().is_empty());
    drop(open);
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(not(target_os = "ios"))]
#[test]
fn without_a_callback_only_ios_has_a_vault() {
    assert_eq!(vault(None).err(), Some(MobileError::SecretStore));
}

#[test]
fn the_vault_callback_is_keyed_by_the_account_id() {
    let secrets = Arc::new(Deletes::default());
    let source = ForeignSource(secrets.clone());
    source.delete_account(AccountId::new(7)).unwrap();
    assert_eq!(*secrets.0.lock().unwrap(), [7]);
}
