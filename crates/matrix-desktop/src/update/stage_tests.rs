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
fn clean_orphans_removes_only_old_gnobbi_new_files() {
    let t = t("orphans");
    let d = t.staging.parent().unwrap();
    let keep = d.join("other.new");
    std::fs::write(&keep, "x").unwrap();
    std::fs::write(&t.staging, "x").unwrap();
    clean_orphans(d, Duration::from_secs(3600));
    assert!(t.staging.exists());
    clean_orphans(d, Duration::ZERO);
    assert!(!t.staging.exists() && keep.exists());
}
