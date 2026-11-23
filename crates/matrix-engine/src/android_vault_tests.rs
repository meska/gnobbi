use super::*;
use std::{cell::Cell, os::unix::fs::PermissionsExt};

/// Stands in for the JNI bridge: XOR plus a tag that binds the AAD; counts `open` calls.
#[derive(Default)]
struct Fake {
    opens: Cell<u32>,
    key_lost: bool,
}
impl Fake {
    fn tag(aad: &[u8]) -> [u8; 16] {
        let mut t = [0u8; 16];
        aad.iter().enumerate().for_each(|(i, b)| t[i % 16] ^= b.wrapping_add(i as u8));
        t
    }
}
impl Sealer for Fake {
    fn seal(&self, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, VaultError> {
        let mut out = vec![7u8; IV_LEN];
        out.extend(plain.iter().map(|b| b ^ 0x5a));
        out.extend(Self::tag(aad));
        Ok(out)
    }
    fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        self.opens.set(self.opens.get() + 1);
        if self.key_lost {
            return Err(VaultError::InvalidItem);
        }
        let (body, tag) = sealed[IV_LEN..].split_at(sealed.len() - IV_LEN - 16);
        if tag != Self::tag(aad) {
            return Err(VaultError::InvalidItem);
        }
        Ok(Zeroizing::new(body.iter().map(|b| b ^ 0x5a).collect()))
    }
}

fn vault(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gnobbi-android-vault-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fsx::create_private_dir(&dir).unwrap();
    dir
}
const RANDOM: [u8; 32] = [0xab; 32];
fn store(n: u32) -> PathBuf {
    PathBuf::from(format!("/data/accounts/{n}/account-1"))
}

#[test]
fn create_then_read_round_trips_in_a_private_93_byte_file() {
    let dir = vault("roundtrip");
    let fake = Fake::default();
    let created = create_in(&dir, &fake, &store(2), &RANDOM).unwrap();
    let file = file_of(&dir, &item_of(&store(2)).unwrap());
    let meta = fs::metadata(&file).unwrap();
    assert_eq!(meta.len(), 93);
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert_eq!(fs::read(&file).unwrap()[0], 1);
    assert!(!fs::read(&file).unwrap().windows(8).any(|w| w == b"abababab"), "plain secret on disk");
    assert_eq!(read_in(&dir, &fake, &store(2)).unwrap().0.as_str(), created.0.as_str());
    assert!(!dir.join("profile-id:accounts_2:account-1.gcm.pending").exists());
}

#[test]
fn second_create_is_refused_and_available_follows_the_file() {
    let dir = vault("twice");
    let fake = Fake::default();
    assert!(available_in(&dir, &store(2)).unwrap());
    create_in(&dir, &fake, &store(2), &RANDOM).unwrap();
    assert!(!available_in(&dir, &store(2)).unwrap());
    assert!(matches!(create_in(&dir, &fake, &store(2), &RANDOM), Err(VaultError::AlreadyExists)));
    assert!(available_in(&dir, &store(3)).unwrap());
}

#[test]
fn read_distinguishes_missing_from_malformed_without_calling_the_bridge() {
    let dir = vault("malformed");
    let fake = Fake::default();
    assert!(matches!(read_in(&dir, &fake, &store(2)), Err(VaultError::Missing)));
    let file = file_of(&dir, &item_of(&store(2)).unwrap());
    fs::write(&file, [1u8; 92]).unwrap();
    assert!(matches!(read_in(&dir, &fake, &store(2)), Err(VaultError::InvalidItem)));
    let mut wrong_version = vec![2u8];
    wrong_version.extend([0u8; 92]);
    fs::write(&file, wrong_version).unwrap();
    assert!(matches!(read_in(&dir, &fake, &store(2)), Err(VaultError::InvalidItem)));
    assert_eq!(fake.opens.get(), 0);
}

#[test]
fn blob_moved_to_another_profile_or_with_lost_key_is_invalid_and_kept() {
    let dir = vault("moved");
    let fake = Fake::default();
    create_in(&dir, &fake, &store(2), &RANDOM).unwrap();
    let from = file_of(&dir, &item_of(&store(2)).unwrap());
    let to = file_of(&dir, &item_of(&store(3)).unwrap());
    fs::copy(&from, &to).unwrap();
    assert!(matches!(read_in(&dir, &fake, &store(3)), Err(VaultError::InvalidItem)));
    let lost = Fake { key_lost: true, ..Fake::default() };
    assert!(matches!(read_in(&dir, &lost, &store(2)), Err(VaultError::InvalidItem)));
    assert!(from.exists(), "a lost key must not delete the blob");
}

#[test]
fn delete_removes_the_blob_and_tolerates_absence() {
    let dir = vault("delete");
    create_in(&dir, &Fake::default(), &store(2), &RANDOM).unwrap();
    delete_in(&dir, &store(2)).unwrap();
    assert!(available_in(&dir, &store(2)).unwrap());
    delete_in(&dir, &store(2)).unwrap();
}

#[test]
fn a_path_without_an_item_name_is_unavailable() {
    let dir = vault("noitem");
    assert!(matches!(create_in(&dir, &Fake::default(), Path::new("/x/other"), &RANDOM), Err(VaultError::Unavailable)));
}

#[test]
fn java_exceptions_map_to_vault_errors() {
    for name in [
        "javax.crypto.AEADBadTagException",
        "android.security.keystore.KeyPermanentlyInvalidatedException",
    ] {
        assert!(matches!(classify(name), VaultError::InvalidItem), "{name}");
    }
    for name in ["java.security.KeyStoreException", "android.security.KeyStoreException", ""] {
        assert!(matches!(classify(name), VaultError::Unavailable), "{name}");
    }
}
