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
}
