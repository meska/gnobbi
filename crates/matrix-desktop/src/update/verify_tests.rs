//! Tests against REAL minisign signatures made by `tests/fixtures/update/gen.sh` with
//! throw-away test keys (secret keys are never stored). No network, injected clock.
use super::*;
use std::time::Duration;

const HOST: &str = "gnobbi.meskatech.net";
/// 2026-12-05T18:40:00Z, the `valid_until` of the fixtures.
const VALID_UNTIL: u64 = 1_796_496_000;

fn fx(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/update")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}
fn fx_str(name: &str) -> String {
    String::from_utf8(fx(name)).unwrap()
}
fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}
fn now() -> SystemTime {
    at(VALID_UNTIL - 10 * 86_400)
}
fn keys() -> UpdateKeys {
    UpdateKeys::parse(&fx_str("keys.pub")).expect("fixture keys")
}
fn check(manifest: &[u8], sig: &[u8], running: u64) -> Result<Candidate, UpdateError> {
    verify_manifest(&keys(), manifest, sig, running, now(), LINUX_APPIMAGE, HOST)
}
/// Manifest `<name>.json` with its own `<name>.minisig`.
fn check_fx(name: &str, running: u64) -> Result<Candidate, UpdateError> {
    check(
        &fx(&format!("{name}.json")),
        &fx(&format!("{name}.minisig")),
        running,
    )
}

// ---------- positive ----------

#[test]
fn valid_manifest_signed_by_current_key_is_accepted() {
    let c = check_fx("valid", 41).expect("valid manifest");
    assert_eq!(
        c,
        Candidate {
            version: "0.1.0".into(),
            build: 42,
            file_name: "Gnobbi-x86_64-42.AppImage".into(),
            sha256: "d2358579a99e69a151932fe9805328e88c226dbf8fdd72cc5aff1085931fad40".into(),
            size: 1000,
        }
    );
}

#[test]
fn signature_by_next_key_is_accepted_rotation() {
    let c = check(&fx("valid.json"), &fx("valid.next.minisig"), 41).expect("rotation");
    assert_eq!(c.build, 42);
}

#[test]
fn single_key_file_works() {
    let k = UpdateKeys::parse(&fx_str("current.pub")).unwrap();
    let r = verify_manifest(
        &k,
        &fx("valid.json"),
        &fx("valid.minisig"),
        41,
        now(),
        LINUX_APPIMAGE,
        HOST,
    );
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn one_second_before_valid_until_is_accepted() {
    let r = verify_manifest(
        &keys(),
        &fx("valid.json"),
        &fx("valid.minisig"),
        41,
        at(VALID_UNTIL - 1),
        LINUX_APPIMAGE,
        HOST,
    );
    assert!(r.is_ok(), "{r:?}");
}

// ---------- key file ----------

#[test]
fn key_file_with_no_keys_is_rejected() {
    assert!(matches!(
        UpdateKeys::parse(""),
        Err(UpdateError::BadKeyFile)
    ));
}

#[test]
fn key_file_with_three_keys_is_rejected() {
    let f = format!(
        "{}{}{}",
        fx_str("current.pub"),
        fx_str("next.pub"),
        fx_str("other.pub")
    );
    assert!(matches!(
        UpdateKeys::parse(&f),
        Err(UpdateError::BadKeyFile)
    ));
}

#[test]
fn key_file_with_undecodable_key_is_rejected() {
    let f = "untrusted comment: minisign public key 0000000000000000\nnot-a-key\n";
    assert!(matches!(UpdateKeys::parse(f), Err(UpdateError::BadKeyFile)));
}

// ---------- signature ----------

#[test]
fn altered_signature_is_invalid_signature() {
    let mut sig = fx_str("valid.minisig").into_bytes();
    // line 2 = signature; flip a base64 char well past the key id (bytes 2..10).
    let line2 = sig.iter().position(|&b| b == b'\n').unwrap() + 1;
    let i = line2 + 40;
    sig[i] = if sig[i] == b'A' { b'B' } else { b'A' };
    assert_eq!(
        check(&fx("valid.json"), &sig, 41),
        Err(UpdateError::InvalidSignature)
    );
}

#[test]
fn altered_trusted_comment_is_invalid_signature() {
    // the global signature covers the trusted comment
    let sig = fx_str("valid.minisig")
        .replace("build=42", "build=43")
        .into_bytes();
    assert_eq!(
        check(&fx("valid.json"), &sig, 41),
        Err(UpdateError::InvalidSignature)
    );
}

#[test]
fn manifest_altered_after_signing_is_invalid_signature() {
    let mut m = fx("valid.json");
    let i = m.windows(5).position(|w| w == b"1000}").unwrap();
    m[i] = b'2'; // size 1000 -> 2000, still valid JSON
    assert_eq!(
        check(&m, &fx("valid.minisig"), 41),
        Err(UpdateError::InvalidSignature)
    );
}

#[test]
fn manifest_without_trailing_newline_is_invalid_signature() {
    let mut m = fx("valid.json");
    assert_eq!(m.pop(), Some(b'\n'));
    assert_eq!(
        check(&m, &fx("valid.minisig"), 41),
        Err(UpdateError::InvalidSignature)
    );
}

#[test]
fn signature_of_unknown_key_is_unknown_key() {
    assert_eq!(
        check(&fx("valid.json"), &fx("valid.other.minisig"), 41),
        Err(UpdateError::UnknownKey)
    );
}

#[test]
fn signature_from_other_key_with_only_next_trusted_is_unknown_key() {
    // current's signature against a keys file that holds only `next`
    let k = UpdateKeys::parse(&fx_str("next.pub")).unwrap();
    let r = verify_manifest(
        &k,
        &fx("valid.json"),
        &fx("valid.minisig"),
        41,
        now(),
        LINUX_APPIMAGE,
        HOST,
    );
    assert_eq!(r, Err(UpdateError::UnknownKey));
}

#[test]
fn garbage_signature_is_malformed_signature() {
    assert_eq!(
        check(&fx("valid.json"), b"not a signature", 41),
        Err(UpdateError::MalformedSignature)
    );
    assert_eq!(
        check(&fx("valid.json"), &[0xff, 0xfe, 0x00], 41),
        Err(UpdateError::MalformedSignature)
    );
    assert_eq!(
        check(&fx("valid.json"), b"", 41),
        Err(UpdateError::MalformedSignature)
    );
}

#[test]
fn legacy_non_prehashed_signature_is_malformed_signature() {
    assert_eq!(
        check(&fx("valid.json"), &fx("valid.legacy.minisig"), 41),
        Err(UpdateError::MalformedSignature)
    );
}

#[test]
fn oversize_manifest_is_too_large() {
    let big = vec![b' '; MAX_MANIFEST_BYTES + 1];
    assert_eq!(
        check(&big, &fx("valid.minisig"), 41),
        Err(UpdateError::TooLarge)
    );
}

#[test]
fn oversize_signature_is_too_large() {
    let big = vec![b'A'; MAX_SIGNATURE_BYTES + 1];
    assert_eq!(
        check(&fx("valid.json"), &big, 41),
        Err(UpdateError::TooLarge)
    );
}

// ---------- signed, but rejected by the rules ----------

#[test]
fn trusted_comment_with_other_build_is_rejected() {
    assert_eq!(
        check_fx("comment_mismatch", 41),
        Err(UpdateError::TrustedComment)
    );
}

#[test]
fn trusted_comment_in_wrong_format_is_rejected() {
    assert_eq!(
        check_fx("comment_format", 41),
        Err(UpdateError::TrustedComment)
    );
    assert_eq!(
        check_fx("comment_zero", 41),
        Err(UpdateError::TrustedComment)
    ); // "build=042"
}

#[test]
fn signed_non_json_is_malformed() {
    assert!(matches!(
        check_fx("notjson", 41),
        Err(UpdateError::Malformed(_))
    ));
}

#[test]
fn signed_manifest_missing_a_required_field_is_malformed() {
    assert!(matches!(
        check_fx("missing_sha", 41),
        Err(UpdateError::Malformed(_))
    ));
}

#[test]
fn unknown_schema_is_unsupported_schema() {
    assert_eq!(
        check_fx("schema2", 41),
        Err(UpdateError::UnsupportedSchema(2))
    );
}

#[test]
fn expired_manifest_is_expired() {
    assert_eq!(check_fx("expired", 41), Err(UpdateError::Expired));
}

#[test]
fn now_equal_to_valid_until_is_expired() {
    let r = verify_manifest(
        &keys(),
        &fx("valid.json"),
        &fx("valid.minisig"),
        41,
        at(VALID_UNTIL),
        LINUX_APPIMAGE,
        HOST,
    );
    assert_eq!(r, Err(UpdateError::Expired));
}

#[test]
fn same_build_is_not_newer() {
    assert_eq!(
        check_fx("valid", 42),
        Err(UpdateError::NotNewer {
            build: 42,
            running: 42
        })
    );
}

#[test]
fn lower_build_is_not_newer_rollback() {
    assert_eq!(
        check_fx("valid", 100),
        Err(UpdateError::NotNewer {
            build: 42,
            running: 100
        })
    );
}

#[test]
fn missing_platform_artifact_is_no_artifact() {
    assert_eq!(check_fx("noartifact", 41), Err(UpdateError::NoArtifact));
    assert_eq!(
        verify_manifest(
            &keys(),
            &fx("valid.json"),
            &fx("valid.minisig"),
            41,
            now(),
            WINDOWS_ZIP,
            HOST
        ),
        Err(UpdateError::NoArtifact)
    );
}

#[test]
fn http_url_is_not_https() {
    assert_eq!(check_fx("http", 41), Err(UpdateError::NotHttps));
}

#[test]
fn url_of_other_domain_is_wrong_host() {
    assert_eq!(check_fx("otherhost", 41), Err(UpdateError::WrongHost));
}

#[test]
fn expected_host_is_compared_not_hardcoded() {
    let r = verify_manifest(
        &keys(),
        &fx("valid.json"),
        &fx("valid.minisig"),
        41,
        now(),
        LINUX_APPIMAGE,
        "mirror.lan",
    );
    assert_eq!(r, Err(UpdateError::WrongHost));
}

#[test]
fn dotdot_and_percent_file_names_are_bad_file_name() {
    assert_eq!(check_fx("dotdot", 41), Err(UpdateError::BadFileName));
    assert_eq!(check_fx("percent", 41), Err(UpdateError::BadFileName));
}

#[test]
fn verify_manifest_never_panics_on_junk() {
    for junk in [&b""[..], b"\0", b"{}", b"\xff\xff", b"[1,2,3]"] {
        assert!(check(junk, &fx("valid.minisig"), 41).is_err());
        assert!(check(&fx("valid.json"), junk, 41).is_err());
    }
}

// ---------- downloaded file ----------

struct Tmp(std::path::PathBuf);
impl Tmp {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let dir = std::env::temp_dir().join(format!("gnobbi-verify-{}-{name}", std::process::id()));
        std::fs::write(&dir, bytes).unwrap();
        Tmp(dir)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn candidate() -> Candidate {
    check_fx("valid", 41).unwrap()
}

#[test]
fn file_with_right_size_and_sha256_is_accepted() {
    let f = Tmp::new("ok", &fx("artifact.bin"));
    assert_eq!(verify_file(&f.0, &candidate()), Ok(()));
}

#[test]
fn same_size_other_content_is_sha256_mismatch() {
    let mut b = fx("artifact.bin");
    b[0] ^= 1;
    let f = Tmp::new("flip", &b);
    assert_eq!(
        verify_file(&f.0, &candidate()),
        Err(UpdateError::Sha256Mismatch)
    );
}

#[test]
fn shorter_file_is_size_mismatch() {
    let mut b = fx("artifact.bin");
    b.truncate(999);
    let f = Tmp::new("short", &b);
    assert_eq!(
        verify_file(&f.0, &candidate()),
        Err(UpdateError::SizeMismatch {
            expected: 1000,
            actual: 999
        })
    );
}

#[test]
fn longer_file_is_size_mismatch() {
    let mut b = fx("artifact.bin");
    b.push(0);
    let f = Tmp::new("long", &b);
    // the read stops at size + 1, so a longer file reports exactly 1001
    assert_eq!(
        verify_file(&f.0, &candidate()),
        Err(UpdateError::SizeMismatch {
            expected: 1000,
            actual: 1001
        })
    );
}

#[test]
fn empty_file_is_size_mismatch() {
    let f = Tmp::new("empty", b"");
    assert_eq!(
        verify_file(&f.0, &candidate()),
        Err(UpdateError::SizeMismatch {
            expected: 1000,
            actual: 0
        })
    );
}

#[test]
fn missing_file_is_io_not_found() {
    let p = std::env::temp_dir().join(format!("gnobbi-verify-{}-absent", std::process::id()));
    assert_eq!(
        verify_file(&p, &candidate()),
        Err(UpdateError::Io(std::io::ErrorKind::NotFound))
    );
}
