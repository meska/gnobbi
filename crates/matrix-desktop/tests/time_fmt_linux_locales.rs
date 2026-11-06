#![cfg(target_os = "linux")]
//! QA: the REAL glibc locales through `time_fmt::system()`. The patterns are read once per process, so each
//! locale runs in a child process of this test binary. A locale that is not installed is skipped (see `locale -a`).
use matrix_desktop::time_fmt::{Patterns, system};
use std::process::Command;

/// Child side: prints the patterns the process read from its own LC_ALL.
#[test]
fn probe_prints_system_patterns() {
    if std::env::var("GNOBBI_LOCALE_PROBE").is_ok() {
        println!("PATTERNS {:?}", system());
    }
}

fn read(locale: &str) -> Option<String> {
    let installed = Command::new("locale").arg("-a").output().ok()?;
    let want = locale.replace("UTF-8", "utf8");
    if !String::from_utf8_lossy(&installed.stdout).lines().any(|l| l == want || l == locale) {
        eprintln!("skip {locale}: not installed");
        return None;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "probe_prints_system_patterns", "--nocapture"])
        .env_clear()
        .env("LC_ALL", locale)
        .env("GNOBBI_LOCALE_PROBE", "1")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().find(|l| l.starts_with("PATTERNS")).map(str::to_owned)
}

#[test]
fn real_locales_give_their_own_patterns() {
    // (locale, date pattern, time pattern, Monday, Sunday)
    let cases = [
        ("it_IT.UTF-8", "dd/MM/yy", "HH:mm", "lun", "dom"),
        ("en_US.UTF-8", "MM/dd/yy", "hh:mm a", "Mon", "Sun"),
        ("en_GB.UTF-8", "dd/MM/yy", "HH:mm", "Mon", "Sun"),
        ("de_DE.UTF-8", "dd.MM.yy", "HH:mm", "Mo", "So"),
        ("ja_JP.UTF-8", "yy年MM月dd日", "HH時mm分", "月", "日"),
    ];
    let mut checked = 0;
    for (loc, date, time, mon, sun) in cases {
        let Some(line) = read(loc) else { continue };
        checked += 1;
        for needle in [format!("date: {date:?}"), format!("time: {time:?}"), format!("{mon:?}"), format!("{sun:?}")] {
            assert!(line.contains(&needle), "{loc}: {needle} not in {line}");
        }
    }
    eprintln!("locales checked: {checked}/5");
}

#[test]
fn c_posix_and_unknown_locales_use_the_fallback() {
    let fb = format!("PATTERNS {:?}", Patterns::fallback());
    for loc in ["C", "POSIX", "C.UTF-8", "xx_XX.UTF-8"] {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "probe_prints_system_patterns", "--nocapture"])
            .env_clear()
            .env("LC_ALL", loc)
            .env("GNOBBI_LOCALE_PROBE", "1")
            .output()
            .unwrap();
        let line = String::from_utf8_lossy(&out.stdout).lines().find(|l| l.starts_with("PATTERNS")).map(str::to_owned);
        assert_eq!(line.as_deref(), Some(fb.as_str()), "{loc}");
    }
}
