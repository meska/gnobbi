use super::stage::*;
use super::{startup, verify::Candidate};
use crate::AppDirs;
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf, sync::Mutex, time::Duration};

struct T { dirs: AppDirs, staging: PathBuf, state: Mutex<UpdateState> }

fn t(name: &str) -> T {
    let root = std::env::temp_dir().join(format!("gnobbi-stage-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dirs = AppDirs { config: root.join("config"), data: root.join("data"), cache: root.join("cache") };
    T { dirs, staging: root.join(".gnobbi.AppImage.new"), state: Mutex::default() }
}

fn cand(body: &[u8], build: u64) -> Candidate {
    let sha256 = Sha256::digest(body).iter().map(|b| format!("{b:02x}")).collect();
    Candidate { version: "1.2".into(), build, file_name: "x".into(), sha256, size: body.len() as u64 }
}

fn script(body: &str) -> Vec<u8> { format!("#!/bin/sh\n{body}\n").into_bytes() }

fn stage(t: &T, body: &[u8], c: &Candidate, timeout: Duration) -> Result<Ready, StageError> {
    stage_with_timeout(&t.dirs, &t.state, c, Cursor::new(body.to_vec()), &t.staging, timeout)
}

const T10: Duration = Duration::from_secs(10);

#[test]
fn failed_restage_preserves_verified_ready_file() {
    let t = t("restage");
    let b = script("echo gnobbi build=7");
    let c = cand(&b, 7);
    let ready = stage(&t, &b, &c, T10).unwrap();
    let second = t.staging.with_file_name(".gnobbi.second.new");
    assert!(matches!(stage_with_timeout(&t.dirs, &t.state, &c, Cursor::new(&b[..5]), &second, T10), Err(StageError::Verify(_))));
    assert!(!second.exists());
    assert_eq!(std::fs::read(&ready.path).unwrap(), b);
    assert_eq!(t.state.lock().unwrap().ready.as_ref(), Some(&ready));
}

#[test]
fn collision_never_modifies_or_deletes_existing_file() {
    let t = t("collision");
    std::fs::write(&t.staging, b"user data").unwrap();
    let b = script("echo gnobbi build=7");
    assert!(stage(&t, &b, &cand(&b, 7), T10).is_err());
    assert_eq!(std::fs::read(&t.staging).unwrap(), b"user data");
}

#[test]
fn ok_is_ready_and_file_stays() {
    let t = t("ok");
    let b = script("echo gnobbi build=7");
    let r = stage(&t, &b, &cand(&b, 7), T10).unwrap();
    assert_eq!((r.version.as_str(), r.build), ("1.2", 7));
    assert!(t.staging.exists());
    assert_eq!(t.state.lock().unwrap().ready.as_ref(), Some(&r));
}

#[test]
fn wrong_hash_deletes_and_is_not_persisted() {
    let t = t("hash");
    let b = script("echo gnobbi build=7");
    let mut c = cand(&b, 7);
    c.sha256 = "0".repeat(64);
    assert!(matches!(stage(&t, &b, &c, T10), Err(StageError::Verify(_))));
    assert!(!t.staging.exists());
    assert!(t.state.lock().unwrap().bad_session.contains(&7));
    assert!(startup::bad_builds(&t.dirs).is_empty());
}

#[test]
fn body_longer_or_shorter_than_size_deletes() {
    let t = t("size");
    let b = script("echo gnobbi build=7");
    let c = cand(&b, 7);
    let mut long = b.clone();
    long.extend_from_slice(&[b'x'; 1000]);
    assert!(matches!(stage(&t, &long, &c, T10), Err(StageError::Verify(_))));
    assert!(!t.staging.exists());
    assert!(matches!(stage(&t, &b[..5], &c, T10), Err(StageError::Verify(_))));
    assert!(!t.staging.exists());
}

#[test]
fn network_error_deletes_partial_file() {
    struct Broken(bool);
    impl std::io::Read for Broken {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0 { return Err(std::io::ErrorKind::ConnectionReset.into()) }
            self.0 = true;
            buf[..4].copy_from_slice(b"part");
            Ok(4)
        }
    }
    let t = t("net");
    let c = cand(b"whatever-longer", 7);
    assert!(matches!(stage_with_timeout(&t.dirs, &t.state, &c, Broken(false), &t.staging, T10), Err(StageError::Io(_))));
    assert!(!t.staging.exists());
    assert!(startup::bad_builds(&t.dirs).is_empty());
}

#[test]
fn self_test_other_build_is_persistent() {
    let t = t("other");
    let b = script("echo gnobbi build=8");
    assert!(matches!(stage(&t, &b, &cand(&b, 7), T10), Err(StageError::SelfTestFailed)));
    assert!(!t.staging.exists());
    assert_eq!(startup::bad_builds(&t.dirs), vec![7]);
    assert!(t.state.lock().unwrap().ready.is_none());
}

#[test]
fn self_test_exit_nonzero_is_persistent() {
    let t = t("exit");
    let b = script("echo gnobbi build=7; exit 3");
    assert!(matches!(stage(&t, &b, &cand(&b, 7), T10), Err(StageError::SelfTestFailed)));
    assert!(!t.staging.exists());
    assert_eq!(startup::bad_builds(&t.dirs), vec![7]);
}

#[test]
fn self_test_timeout_deletes_and_is_memory_only() {
    let t = t("timeout");
    let b = script("exec sleep 30");
    assert!(matches!(stage(&t, &b, &cand(&b, 7), Duration::from_millis(300)), Err(StageError::SelfTestTimeout)));
    assert!(!t.staging.exists());
    assert!(t.state.lock().unwrap().bad_session.contains(&7));
    assert!(startup::bad_builds(&t.dirs).is_empty());
}

#[test]
fn invalid_hash_never_executes_candidate() {
    let t = t("no-exec");
    let marker = t.staging.with_file_name("executed");
    let b = script(&format!("touch '{}'; echo gnobbi build=7", marker.display()));
    let mut c = cand(&b, 7);
    c.sha256 = "0".repeat(64);
    assert!(matches!(stage(&t, &b, &c, T10), Err(StageError::Verify(_))));
    assert!(!marker.exists());
    assert!(!t.staging.exists());
}

#[test]
fn collision_with_ready_preserves_first_attempt() {
    let t = t("ready-collision");
    let b = script("echo gnobbi build=7");
    let c = cand(&b, 7);
    let ready = stage(&t, &b, &c, T10).unwrap();
    let other = Mutex::default();
    assert!(matches!(stage_with_timeout(&t.dirs, &other, &c, Cursor::new(&b), &t.staging, T10), Err(StageError::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists));
    assert_eq!(std::fs::read(&ready.path).unwrap(), b);
    assert_eq!(t.state.lock().unwrap().ready.as_ref(), Some(&ready));
    assert_eq!((ready.size, &ready.sha256), (c.size, &c.sha256));
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        w.start_file(*name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn zip_cand(zip: &[u8], build: u64) -> Candidate {
    Candidate { file_name: "gnobbi-1-windows-x64.zip".into(), ..cand(zip, build) }
}

fn leftovers(t: &T) -> Vec<String> {
    std::fs::read_dir(t.staging.parent().unwrap()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(".gnobbi")).collect()
}

#[test]
fn windows_zip_is_verified_then_only_the_exe_is_staged() {
    let t = t("zip-ok");
    let exe = script("echo gnobbi build=7");
    let z = zip_of(&[("readme.txt", b"hi"), ("gnobbi.exe", &exe)]);
    let ready = stage(&t, &z, &zip_cand(&z, 7), T10).unwrap();
    assert_eq!(std::fs::read(&ready.path).unwrap(), exe);
    assert_eq!((ready.size, ready.sha256), (exe.len() as u64, cand(&exe, 7).sha256), "Ready describes the exe, not the zip");
    assert_eq!(leftovers(&t), [t.staging.file_name().unwrap().to_string_lossy()], "the zip is gone");
}

#[test]
fn windows_zip_failures_leave_nothing_and_never_run_it() {
    let t = t("zip-bad");
    let exe = script("echo gnobbi build=7");
    let z = zip_of(&[("gnobbi.exe", &exe)]);
    let mut wrong_hash = zip_cand(&z, 7);
    wrong_hash.sha256 = cand(b"other", 7).sha256;
    assert!(matches!(stage(&t, &z, &wrong_hash, T10), Err(StageError::Verify(_))));
    let no_exe = zip_of(&[("nested/gnobbi.exe", &exe), ("other.exe", &exe)]);
    assert!(matches!(stage(&t, &no_exe, &zip_cand(&no_exe, 7), T10), Err(StageError::BadZip)));
    assert!(matches!(stage(&t, b"not a zip", &zip_cand(b"not a zip", 7), T10), Err(StageError::BadZip)));
    assert_eq!(leftovers(&t), Vec::<String>::new());
    assert!(t.state.lock().unwrap().ready.is_none());
}

#[test]
fn declared_size_above_the_absolute_cap_is_refused_before_any_read() {
    // 512 MiB is too big to feed in a test: the refusal happens on the declared size, before the file is created.
    let t = t("too-large");
    let b = script("echo gnobbi build=7");
    let c = Candidate { size: MAX_ARTIFACT_BYTES + 1, ..cand(&b, 7) };
    assert!(matches!(stage(&t, &b, &c, T10), Err(StageError::TooLarge)));
    assert_eq!(leftovers(&t), Vec::<String>::new());
}

#[test]
fn body_longer_than_declared_is_stopped_while_receiving() {
    struct Endless(u64);
    impl std::io::Read for Endless {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            self.0 += out.len() as u64;
            assert!(self.0 < 1 << 20, "read far past the declared size");
            out.fill(b'x');
            Ok(out.len())
        }
    }
    let t = t("endless");
    let b = script("echo gnobbi build=7");
    assert!(matches!(stage_with_timeout(&t.dirs, &t.state, &cand(&b, 7), Endless(0), &t.staging, T10), Err(StageError::Verify(_))));
    assert_eq!(leftovers(&t), Vec::<String>::new());
}

#[test]
fn apk_is_verified_but_never_executed() {
    let t = t("apk");
    let apk = b"PK\x03\x04 not an executable";
    let c = Candidate { file_name: "gnobbi-android-arm64-42.apk".into(), ..cand(apk, 7) };
    let r = stage(&t, apk, &c, T10).unwrap();
    assert_eq!(std::fs::read(&r.path).unwrap(), apk);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&r.path).unwrap().permissions().mode() & 0o111, 0, "no exec bit, no --self-test");
    let t = self::t("apk-bad");
    let bad = Candidate { sha256: "0".repeat(64), ..c };
    assert!(matches!(stage(&t, apk, &bad, T10), Err(StageError::Verify(_))));
    assert!(!t.staging.exists());
}
