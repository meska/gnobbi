//! «Esporta chiavi»: room keys in the standard Element file (MEGOLM SESSION DATA, AES-CTR +
//! HMAC-SHA256, PBKDF2-SHA512 500 000 rounds). Backend runtime only; never log the passphrase,
//! the path or the keys.
use crate::fsx;
use matrix_sdk::Client;
use std::{
    io::{Cursor, Write},
    path::Path,
    time::Duration,
};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyExportError {
    /// Local unlock phrase of a legacy profile is wrong or missing.
    WrongPhrase,
    /// The file password does not open the file (import), or it is not a key export.
    WrongPassword,
    NoKeys,
    NotWritable,
    Deadline,
    Failed,
}

/// Exports every room key to `dest` (0600) and returns how many. `scratch` is a private file
/// beside the account store: the export is written and verified there first, so a failed or
/// empty export never touches `dest` (the macOS sandbox also forbids sibling temp files).
pub async fn export_room_keys(
    client: &Client,
    scratch: &Path,
    dest: &Path,
    passphrase: &str,
) -> Result<usize, KeyExportError> {
    let result = tokio::time::timeout(Duration::from_secs(120), async {
        fsx::remove_pending(scratch).map_err(|_| KeyExportError::Failed)?;
        fsx::private_file(std::fs::OpenOptions::new().write(true).create_new(true))
            .open(scratch)
            .map_err(|_| KeyExportError::Failed)?;
        client
            .encryption()
            .export_room_keys(scratch.to_owned(), passphrase, |_| true)
            .await
            .map_err(|_| KeyExportError::Failed)?;
        let bytes = std::fs::read(scratch).map_err(|_| KeyExportError::Failed)?;
        let passphrase = Zeroizing::new(passphrase.to_owned());
        // Round trip before touching `dest`; the decrypted keys are dropped inside the closure.
        let (count, bytes) = tokio::task::spawn_blocking(move || {
            matrix_sdk_crypto::decrypt_room_key_export(Cursor::new(&bytes), &passphrase)
                .map(|keys| (keys.len(), bytes))
                .map_err(|_| KeyExportError::Failed)
        })
        .await
        .map_err(|_| KeyExportError::Failed)??;
        if count == 0 {
            return Err(KeyExportError::NoKeys);
        }
        write_dest(dest, &bytes)?;
        Ok(count)
    })
    .await
    .unwrap_or(Err(KeyExportError::Deadline));
    let _ = fsx::remove_pending(scratch);
    result
}

/// Owner-only mode is set before any byte is written, also on a pre-existing 0644 file.
fn write_dest(dest: &Path, bytes: &[u8]) -> Result<(), KeyExportError> {
    use std::io::ErrorKind::*;
    let mut file = fsx::private_file(std::fs::OpenOptions::new().write(true).create(true))
        .open(dest)
        .map_err(|e| match e.kind() {
            PermissionDenied | NotFound | ReadOnlyFilesystem => KeyExportError::NotWritable,
            _ => KeyExportError::Failed,
        })?;
    let written = (|| {
        #[cfg(unix)]
        file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        file.set_len(0)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(dest);
        match e.kind() {
            PermissionDenied | NotFound | ReadOnlyFilesystem => KeyExportError::NotWritable,
            _ => KeyExportError::Failed,
        }
    })
}

/// Imports an exported file; returns (imported, total). Used by tests and the round trip only:
/// there is no import screen.
pub async fn import_room_keys(
    client: &Client,
    src: &Path,
    passphrase: &str,
) -> Result<(usize, usize), KeyExportError> {
    use matrix_sdk::encryption::RoomKeyImportError as E;
    let result = tokio::time::timeout(
        Duration::from_secs(120),
        client
            .encryption()
            .import_room_keys(src.to_owned(), passphrase),
    )
    .await
    .map_err(|_| KeyExportError::Deadline)?;
    result
        .map(|r| (r.imported_count, r.total_count))
        .map_err(|e| match e {
            E::Export(_) => KeyExportError::WrongPassword,
            _ => KeyExportError::Failed,
        })
}

/// Local-phrase check (blocking KDF) plus the export, as the verification service runs it.
pub(crate) async fn run_export(
    client: &Client,
    account_id: matrix_domain::AccountId,
    account_dir: Option<&Path>,
    dest: &Path,
    passphrase: &str,
    phrase: Option<&str>,
) -> Result<usize, KeyExportError> {
    check_phrase_async(account_dir, account_id, phrase).await?;
    let dir = account_dir.ok_or(KeyExportError::Failed)?;
    export_room_keys(client, &dir.join("key-export.pending"), dest, passphrase).await
}

pub(crate) async fn check_phrase_async(
    account_dir: Option<&Path>,
    account_id: matrix_domain::AccountId,
    phrase: Option<&str>,
) -> Result<(), KeyExportError> {
    let dir = account_dir.ok_or(KeyExportError::Failed)?.to_owned();
    let phrase = phrase.map(|p| Zeroizing::new(p.to_owned()));
    tokio::task::spawn_blocking(move || {
        check_phrase(&dir, account_id, phrase.as_ref().map(|p| p.as_str()))
    })
    .await
    .unwrap_or(Err(KeyExportError::Failed))
}

/// Legacy profiles (no `protection-mode` beside the account dir) re-enter the local unlock
/// phrase before an export or a recovery-key reset; vault profiles have none to ask for.
pub(crate) fn check_phrase(
    account_dir: &Path,
    account_id: matrix_domain::AccountId,
    phrase: Option<&str>,
) -> Result<(), KeyExportError> {
    let legacy = account_dir
        .parent()
        .is_some_and(|root| !root.join("protection-mode").exists());
    if !legacy {
        return Ok(());
    }
    let phrase = phrase.ok_or(KeyExportError::WrongPhrase)?;
    crate::persistence::AccountStore::unseal_at(account_dir, account_id, phrase)
        .map(drop)
        .map_err(|_| KeyExportError::WrongPhrase)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gnobbi-keyexport-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        fsx::create_private_dir(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn dest_is_0600_new_or_overwritten_and_holds_exactly_the_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("mode");
        let (fresh, old) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&old, b"previous longer content").unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o644)).unwrap();
        for path in [&fresh, &old] {
            write_dest(path, b"MEGOLM").unwrap();
            assert_eq!(std::fs::read(path).unwrap(), b"MEGOLM");
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_folder_is_not_writable() {
        let dir = temp("missing");
        assert_eq!(
            write_dest(&dir.join("no/such/dir/k.txt"), b"x"),
            Err(KeyExportError::NotWritable)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn read_only_folder_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("ro");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = write_dest(&dir.join("k.txt"), b"x");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        // root ignores the mode bits (the CT runs as root): there the write simply succeeds.
        assert!(matches!(result, Err(KeyExportError::NotWritable) | Ok(())));
    }

    #[test]
    fn element_file_round_trips_and_wrong_password_fails() {
        let bytes =
            matrix_sdk_crypto::encrypt_room_key_export(&[], "correct horse battery", 1000).unwrap();
        assert!(bytes.starts_with("-----BEGIN MEGOLM SESSION DATA-----"));
        assert!(!bytes.contains("correct horse"));
        let keys = matrix_sdk_crypto::decrypt_room_key_export(
            Cursor::new(bytes.as_bytes()),
            "correct horse battery",
        )
        .unwrap();
        assert_eq!(keys.len(), 0);
        assert!(
            matrix_sdk_crypto::decrypt_room_key_export(
                Cursor::new(bytes.as_bytes()),
                "wrong password here"
            )
            .is_err()
        );
    }

    // ---- export_room_keys against a real (offline) client ----

    const PASS: &str = "correct horse battery";
    const ROOM: &str = "!secretroom:x.test";

    async fn client() -> Client {
        let client = Client::builder()
            .homeserver_url("http://127.0.0.1:9")
            .server_versions([matrix_sdk::ruma::api::MatrixVersion::V1_0])
            .build()
            .await
            .unwrap();
        let session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(serde_json::json!({
                "user_id": "@a:x.test", "device_id": "DEV", "access_token": "tok"
            }))
            .unwrap();
        client.restore_session(session).await.unwrap();
        client
    }

    /// A real megolm key in Element file form; returns (file, session_id, exported session key).
    fn key_file(pass: &str) -> (std::path::PathBuf, String, String) {
        use matrix_sdk_crypto::vodozemac::{megolm::{GroupSession, InboundGroupSession}, olm::Account};
        let group = GroupSession::new(Default::default());
        let account = Account::new();
        let session_key = InboundGroupSession::new(&group.session_key(), Default::default())
            .export_at_first_known_index()
            .to_base64();
        let key: matrix_sdk_crypto::olm::ExportedRoomKey = serde_json::from_value(serde_json::json!({
            "algorithm": "m.megolm.v1.aes-sha2", "room_id": ROOM,
            "sender_key": account.curve25519_key().to_base64(),
            "session_id": group.session_id(), "session_key": session_key,
            "sender_claimed_keys": {"ed25519": account.ed25519_key().to_base64()},
            "forwarding_curve25519_key_chain": []
        }))
        .unwrap();
        // unique dir: tests run in parallel and `temp` wipes its directory
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let file = temp(&format!("src{n}")).join("in.txt");
        std::fs::write(&file, matrix_sdk_crypto::encrypt_room_key_export(&[key], pass, 1000).unwrap()).unwrap();
        (file, group.session_id(), session_key)
    }

    async fn client_with_key() -> (Client, String, String) {
        let client = client().await;
        let (file, id, key) = key_file("import pass phrase");
        let r = import_room_keys(&client, &file, "import pass phrase").await.unwrap();
        assert_eq!(r, (1, 1));
        (client, id, key)
    }

    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_keys_is_reported_and_leaves_dest_and_scratch_alone() {
        let dir = temp("nokeys");
        let (scratch, dest) = (dir.join("key-export.pending"), dir.join("out.txt"));
        let r = export_room_keys(&client().await, &scratch, &dest, PASS).await;
        assert_eq!(r, Err(KeyExportError::NoKeys));
        assert!(!dest.exists() && !scratch.exists());
        // An older export at `dest` survives a failed run.
        std::fs::write(&dest, b"old").unwrap();
        let r = export_room_keys(&client().await, &scratch, &dest, PASS).await;
        assert_eq!(r, Err(KeyExportError::NoKeys));
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn export_round_trips_is_private_and_has_no_plaintext_on_disk() {
        let (client, id, session_key) = client_with_key().await;
        let dir = temp("roundtrip");
        let (scratch, dest) = (dir.join("key-export.pending"), dir.join("out.txt"));
        assert_eq!(export_room_keys(&client, &scratch, &dest, PASS).await, Ok(1));
        assert!(!scratch.exists(), "scratch file must be removed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let bytes = std::fs::read(&dest).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.starts_with("-----BEGIN MEGOLM SESSION DATA-----"));
        for secret in [PASS, ROOM, id.as_str(), session_key.as_str(), "m.megolm"] {
            assert!(!text.contains(secret), "plaintext on disk: {secret}");
            assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
        }
        // Correct password opens it; wrong, empty and near-miss passwords do not, with no keys out.
        let keys = matrix_sdk_crypto::decrypt_room_key_export(Cursor::new(&bytes), PASS).unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].session_id, id);
        for bad in ["wrong password here", "", "correct horse batterY"] {
            assert!(matrix_sdk_crypto::decrypt_room_key_export(Cursor::new(&bytes), bad).is_err());
        }
        // Import side: wrong password is WrongPassword, right one reads the file back.
        assert_eq!(import_room_keys(&client, &dest, "wrong password here").await, Err(KeyExportError::WrongPassword));
        assert_eq!(import_room_keys(&client, &dest, PASS).await.map(|r| r.1), Ok(1));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn truncated_or_foreign_file_is_a_clear_error_not_a_panic() {
        let (client, ..) = client_with_key().await;
        let dir = temp("garbage");
        let (scratch, dest) = (dir.join("p"), dir.join("out.txt"));
        export_room_keys(&client, &scratch, &dest, PASS).await.unwrap();
        let text = std::fs::read_to_string(&dest).unwrap();
        let cut = dir.join("cut.txt");
        std::fs::write(&cut, &text[..text.len() / 2]).unwrap();
        let junk = dir.join("junk.txt");
        std::fs::write(&junk, "hello, not a key file").unwrap();
        for f in [&cut, &junk] {
            assert!(matches!(
                import_room_keys(&client, f, PASS).await,
                Err(KeyExportError::WrongPassword | KeyExportError::Failed)
            ));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unwritable_destination_fails_cleanly_and_removes_scratch() {
        let (client, ..) = client_with_key().await;
        let dir = temp("unwritable");
        let scratch = dir.join("key-export.pending");
        let r = export_room_keys(&client, &scratch, &dir.join("no/such/dir/k.txt"), PASS).await;
        assert_eq!(r, Err(KeyExportError::NotWritable));
        assert!(!scratch.exists());
        // A directory as destination is not a file either.
        let r = export_room_keys(&client, &scratch, &dir, PASS).await;
        assert!(matches!(r, Err(KeyExportError::NotWritable | KeyExportError::Failed)));
        assert!(!scratch.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn logs_and_errors_never_carry_passphrase_keys_or_paths() {
        let sink = Sink::default();
        let writer = sink.clone();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::TRACE)
                .with_writer(move || writer.clone())
                .finish(),
        );
        let (client, id, session_key) = client_with_key().await;
        let dir = temp("leaks-SECRETDIR");
        let (scratch, dest) = (dir.join("key-export.pending"), dir.join("out-SECRETNAME.txt"));
        let mut errors = vec![
            format!("{:?}", export_room_keys(&client, &scratch, &dir.join("x/y"), PASS).await),
            format!("{:?}", export_room_keys(&client, &scratch, &dest, PASS).await),
            format!("{:?}", import_room_keys(&client, &dest, "wrong password here").await),
            format!("{:?}", export_room_keys(&self::client().await, &scratch, &dest, PASS).await),
        ];
        errors.push(format!("{:?}", check_phrase(&dir, matrix_domain::AccountId::new(1), Some("hunter2 phrase"))));
        let logs = String::from_utf8_lossy(&sink.0.lock().unwrap()).to_string();
        for out in errors.iter().chain([&logs]) {
            for secret in [PASS, "wrong password here", "hunter2 phrase", ROOM, id.as_str(), session_key.as_str(), "SECRETDIR", "SECRETNAME"] {
                assert!(!out.contains(secret), "leaked {secret:?} in {out}");
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
