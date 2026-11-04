//! «Informazioni su Gnobbi»: build values come from the build, the copied text is harmless,
//! the drawer row is not permanently styled as a link. No display needed.
use matrix_desktop::build_info::{BUILD_INFO, THIRD_PARTY_NOTICES};
use std::path::{Path, PathBuf};
use std::process::Command;

const ABOUT: &str = include_str!("../src/about_view.rs");
const MENU: &str = include_str!("../src/menu_view.rs");
const BUILD_INFO_SRC: &str = include_str!("../src/build_info.rs");
const WORKSPACE_TOML: &str = include_str!("../../../Cargo.toml");
const CRATE_TOML: &str = include_str!("../Cargo.toml");

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("about-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Compiles build.rs and runs it with `cwd` as working directory; returns its `cargo:` lines.
fn run_build_script(cwd: &Path, env: &[(&str, &str)]) -> String {
    let bin = cwd.join("build-script");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs");
    let ok = Command::new(rustc)
        .args(["--edition", "2021", "-o"])
        .arg(&bin)
        .arg(src)
        .status()
        .unwrap()
        .success();
    assert!(ok, "build.rs compiles on its own");
    let mut cmd = Command::new(&bin);
    cmd.current_dir(cwd)
        .env_remove("GNOBBI_BUILD")
        .env_remove("SOURCE_DATE_EPOCH")
        .env_remove("GIT_DIR")
        // Never let git walk up into a repository around the scratch dir.
        .env("GIT_CEILING_DIRECTORIES", cwd.parent().unwrap());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "build.rs failed: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

fn line(out: &str, key: &str) -> String {
    let p = format!("cargo:rustc-env={key}=");
    out.lines().find_map(|l| l.strip_prefix(&p)).unwrap_or_else(|| panic!("{key} missing in {out}")).into()
}

#[test]
fn version_has_one_source_and_no_hand_written_copy() {
    let ws = WORKSPACE_TOML.lines().find_map(|l| l.strip_prefix("version = \"")).unwrap();
    let ws = ws.trim_end_matches('"');
    assert_eq!(BUILD_INFO.version, ws, "BuildInfo.version is the workspace version");
    assert!(CRATE_TOML.contains("version.workspace = true"));
    for (name, src) in [("about_view.rs", ABOUT), ("menu_view.rs", MENU), ("build_info.rs", BUILD_INFO_SRC)] {
        let code: String = src.split("#[cfg(test)]").next().unwrap().into();
        assert!(!code.contains(ws), "{name} hardcodes the version {ws}");
    }
    assert!(MENU.contains("BUILD_INFO.version") && ABOUT.contains("BUILD_INFO.version"));
}

#[test]
fn shown_values_are_the_compiled_ones() {
    assert_eq!(BUILD_INFO.build, env!("GNOBBI_BUILD_NUMBER"));
    assert_eq!(BUILD_INFO.git_sha, env!("GNOBBI_GIT_SHA"));
    let shown = [BUILD_INFO.version, BUILD_INFO.build, BUILD_INFO.git_sha, BUILD_INFO.build_date, BUILD_INFO.matrix_sdk];
    assert!(shown.iter().all(|v| !v.is_empty() && !v.contains(char::is_whitespace)), "{shown:?}");
    let date = BUILD_INFO.build_date.as_bytes();
    assert!(date.len() == 10 && date[4] == b'-' && date[7] == b'-' || BUILD_INFO.build_date == "sconosciuto");
    // The page reads every field from BUILD_INFO, never from a literal.
    for f in ["version", "build", "git_sha", "build_date", "matrix_sdk"] {
        assert!(ABOUT.contains(&format!("BUILD_INFO.{f}")), "About page does not show {f} from BUILD_INFO");
    }
}

#[test]
fn build_script_without_git_falls_back_and_honours_overrides() {
    let dir = scratch("nogit");
    let out = run_build_script(&dir, &[("SOURCE_DATE_EPOCH", "1700000000")]);
    assert_eq!(line(&out, "GNOBBI_BUILD_NUMBER"), "sconosciuto");
    assert_eq!(line(&out, "GNOBBI_GIT_SHA"), "sconosciuto");
    assert_eq!(line(&out, "GNOBBI_MATRIX_SDK_VERSION"), "sconosciuto", "no Cargo.lock reachable");
    assert_eq!(line(&out, "GNOBBI_BUILD_DATE"), "2023-11-14");
    // GNOBBI_BUILD wins over git, an empty one does not.
    let out = run_build_script(&dir, &[("GNOBBI_BUILD", "77"), ("SOURCE_DATE_EPOCH", "0")]);
    assert_eq!(line(&out, "GNOBBI_BUILD_NUMBER"), "77");
    assert_eq!(line(&out, "GNOBBI_BUILD_DATE"), "1970-01-01");
    let out = run_build_script(&dir, &[("GNOBBI_BUILD", "")]);
    assert_eq!(line(&out, "GNOBBI_BUILD_NUMBER"), "sconosciuto");
    // Leap day.
    let out = run_build_script(&dir, &[("SOURCE_DATE_EPOCH", "1709164800")]);
    assert_eq!(line(&out, "GNOBBI_BUILD_DATE"), "2024-02-29");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn build_script_in_a_git_checkout_reads_head() {
    let dir = scratch("git");
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(args)
            .current_dir(&dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ok
    };
    if !git(&["init", "-q"]) {
        return; // no git on this machine: the no-git test above covers the fallback
    }
    assert!(git(&["commit", "-q", "--allow-empty", "-m", "a"]));
    assert!(git(&["commit", "-q", "--allow-empty", "-m", "b"]));
    let sha = Command::new("git").args(["rev-parse", "--short", "HEAD"]).current_dir(&dir).output().unwrap();
    let sha = String::from_utf8(sha.stdout).unwrap().trim().to_string();
    let out = run_build_script(&dir, &[]);
    assert_eq!(line(&out, "GNOBBI_GIT_SHA"), sha);
    assert_eq!(line(&out, "GNOBBI_BUILD_NUMBER"), "2", "commit count");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn copied_text_has_the_expected_shape_and_no_personal_data() {
    let t = BUILD_INFO.copy_text("Debian GNU/Linux 12 (x86_64)");
    assert_eq!(
        t,
        format!(
            "Gnobbi {}\nBuild: {} ({}, {})\nSistema: Debian GNU/Linux 12 (x86_64)\nMatrix SDK: {}\n",
            BUILD_INFO.version, BUILD_INFO.build, BUILD_INFO.git_sha, BUILD_INFO.build_date, BUILD_INFO.matrix_sdk
        )
    );
    // The text takes only the system string; nothing account-like can get in, and no field is a URL.
    let real = BUILD_INFO.copy_text(&matrix_desktop::build_info::system_version());
    let mut forbidden: Vec<String> = ["@", "://", "/Users/", "/home/", "/root", "token", "password"].map(Into::into).into();
    for k in ["USER", "USERNAME", "LOGNAME", "HOME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(k) {
            if v.len() > 2 && v != "/" && v != "root" {
                forbidden.push(v);
            }
        }
    }
    let first_line_free = real.replacen("Build: ", "", 1);
    for p in &forbidden {
        let hits = first_line_free.lines().filter(|l| !l.starts_with("Sistema:")).any(|l| l.contains(p.as_str()));
        assert!(!hits, "{p:?} found in copied text {real:?}");
    }
    assert!(!real.contains("://") && !real.contains('@'), "{real:?}");
}

#[test]
fn about_page_does_not_touch_account_data() {
    let code: String = ABOUT.lines().filter(|l| !l.trim_start().starts_with("//")).collect();
    for bad in ["verification", "account", "matrix_id", "user_id", "access_token", "homeserver", "prefs_dirs.as_ref().map_or_else(|| ", "std::env::var(\"HOME"] {
        assert!(!code.contains(bad), "about_view.rs references {bad:?}");
    }
}

#[test]
fn no_link_points_to_a_non_public_repository() {
    let private = ["gitea", "192.", "10.", ".lan", "internal", "meskatech.", "localhost", "127.0.0.1", "meska/gnobbi"];
    for (name, text) in [("about_view.rs", ABOUT), ("menu_view.rs", MENU), ("THIRD_PARTY_NOTICES.md", THIRD_PARTY_NOTICES)] {
        for p in private {
            assert!(!text.to_lowercase().contains(p), "{name} mentions {p}");
        }
    }
    // Every URL in the page itself is https and on the project's public domains.
    for w in ABOUT.split(|c: char| c.is_whitespace() || c == '"').filter(|w| w.contains("://")) {
        assert!(w.starts_with("https://gnobbi.com") || w.starts_with("https://gnobbi.chat"), "unexpected link {w}");
    }
    // Every notice URL has a host (the one http:// link, rust-block on github.com, is public).
    for w in THIRD_PARTY_NOTICES.split_whitespace().filter(|w| w.contains("://")) {
        assert!(w.starts_with("https://") || w.starts_with("http://github.com/"), "unexpected link {w}");
    }
}

#[test]
fn drawer_version_row_looks_clickable_only_on_hover_or_focus() {
    let start = MENU.find("fn render_version_line").expect("version row");
    let body = &MENU[start..MENU[start..].find("\n    }\n").map_or(MENU.len(), |e| start + e)];
    assert!(body.contains(".hover("), "hover feedback is missing");
    assert!(body.contains("focused"), "keyboard focus ring is missing");
    for permanent in [".cursor_pointer()", "underline", "text_decoration", "t.accent))\n            .child", ".bg(rgb(t.border))\n            .text_size"] {
        assert!(!body.contains(permanent), "permanent link styling {permanent:?}");
    }
    // Text colour at rest is the quiet secondary one; accent colour only appears in the focus border.
    assert!(body.matches("t.accent").count() == 1, "accent used outside the focus ring");
}

#[test]
fn about_actions_match_the_row_count_and_have_text_names() {
    let rows: usize = ABOUT.lines().find_map(|l| l.strip_prefix("const ABOUT_ROWS: usize = ")).unwrap().trim_end_matches(';').parse().unwrap();
    let render = &ABOUT[ABOUT.find("fn render_about").unwrap()..];
    let actions: Vec<&str> = render.lines().filter_map(|l| l.trim().strip_prefix(".child(action(")).collect();
    assert_eq!(actions.len(), rows, "{actions:?}");
    for (i, a) in actions.iter().enumerate() {
        assert!(a.starts_with(&format!("{i}, \"about-")), "row {i}: {a}");
    }
    for name in ["‹ Indietro", "Apri licenze", "Licenze di terze parti", "Copia informazioni", "Chiudi"] {
        assert!(render.contains(&format!("\"{name}\"")), "{name} missing");
    }
    // Every information row gets its label as text (and element id) from one string.
    assert!(render.contains(".id(label)") && render.contains(".child(label)"));
}
