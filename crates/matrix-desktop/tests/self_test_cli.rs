//! `gnobbi --self-test`: prints the build, exits 0 in < 1 s and leaves the profile untouched,
//! even when an update marker is present.
use std::{path::Path, process::Command, time::{Duration, Instant}};

fn count(p: &Path) -> usize {
    std::fs::read_dir(p).map(|d| d.flatten().map(|e| 1 + count(&e.path())).sum()).unwrap_or(0)
}

#[test]
fn self_test_prints_build_without_touching_files() {
    let home = std::env::temp_dir().join(format!("gnobbi-selftest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let data = home.join(".local/share/gnobbi");
    std::fs::create_dir_all(&data).unwrap();
    let marker = data.join("update-pending.json");
    std::fs::write(&marker, r#"{"from_build":1,"to_build":2,"attempts":2}"#).unwrap();
    let before = (count(&home), std::fs::read(&marker).unwrap());
    let t = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_gnobbi"))
        .arg("--self-test")
        .env_clear()
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .output()
        .unwrap();
    assert!(t.elapsed() < Duration::from_secs(1));
    assert!(out.status.success());
    let line = String::from_utf8(out.stdout).unwrap();
    assert_eq!(line.trim(), format!("gnobbi build={}", matrix_desktop::build_info::BUILD_INFO.build));
    assert_eq!((count(&home), std::fs::read(&marker).unwrap()), before);
    let _ = std::fs::remove_dir_all(&home);
}
