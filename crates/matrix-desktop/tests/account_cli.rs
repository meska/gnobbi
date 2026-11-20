//! `gnobbi account …` as a script sees it: the real binary, a private HOME, no TTY, no keyring and
//! no server. Contract: the ACCOUNT_CLI notes §3 (exit codes), §5 (JSON), §7 (no secrets).
//! Flows against a real homeserver and keyring: the account_cli_e2e script.
#![cfg(unix)]
use serde_json::Value;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// A sentinel that must never show up in any output or file.
const SECRET: &str = "s3cr3t-Sentinel-7f3a91";

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!("gnobbi-clitest-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Home(p)
    }
    fn catalog(&self) -> PathBuf {
        self.0.join("config/gnobbi/accounts.json")
    }
    fn write_catalog(&self, json: &str) {
        let p = self.catalog();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, json).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    /// Every path under HOME, for "nothing was created" and "no secret on disk" checks.
    fn files(&self) -> Vec<PathBuf> {
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                out.push(e.path());
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    walk(&e.path(), out);
                }
            }
        }
        let mut v = vec![];
        walk(&self.0, &mut v);
        v
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    code: i32,
    out: String,
    err: String,
}
impl Run {
    fn json(&self) -> Value {
        assert!(self.err.is_empty(), "--json leaves stderr empty, got: {}", self.err);
        serde_json::from_str(self.out.trim_end()).unwrap_or_else(|e| panic!("stdout is not one JSON object ({e}): {}", self.out))
    }
}

fn gnobbi(home: &Home, args: &[&str], stdin: Option<&[u8]>, env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gnobbi"));
    cmd.arg("account").args(args).env_clear();
    cmd.env("HOME", &home.0)
        .env("XDG_CONFIG_HOME", home.0.join("config"))
        .env("XDG_DATA_HOME", home.0.join("data"))
        .env("XDG_CACHE_HOME", home.0.join("cache"))
        .env("XDG_RUNTIME_DIR", home.0.join("run"))
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}/no-bus", home.0.display()))
        .envs(env.iter().copied());
    // Piped or null: never a terminal, exactly like CI.
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    if let Some(bytes) = stdin {
        let mut pipe = child.stdin.take().unwrap();
        let bytes = bytes.to_vec();
        // A thread: the child may exit without reading (big input), a blocking write must not hang us.
        std::thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let start = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(60) {
            child.kill().unwrap();
            panic!("gnobbi account {args:?} hung (a CI run must never wait for input)");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let o = child.wait_with_output().unwrap();
    let r = Run { code: o.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&o.stdout).into(), err: String::from_utf8_lossy(&o.stderr).into() };
    assert!(!r.out.contains(SECRET) && !r.err.contains(SECRET), "secret printed: {args:?}");
    // `pw` is the test's own input file.
    for f in home.files().iter().filter(|f| f.is_file() && f.file_name().is_some_and(|n| n != "pw")) {
        let bytes = std::fs::read(f).unwrap_or_default();
        assert!(!bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()), "secret written to {f:?}");
    }
    r
}

const ONE_ACCOUNT: &str = r#"{"version":1,"next_id":2,"accounts":[{"id":1,"dir":"accounts/1","user_id":"@a:example.org","homeserver":"https://example.org","label":"a"}]}"#;

fn usage_exit(r: &Run, what: &str) {
    assert_eq!(r.code, 2, "{what}: stdout={:?} stderr={:?}", r.out, r.err);
    assert!(r.err.starts_with("gnobbi: "), "{what}: one `gnobbi: <msg>` line on stderr, got {:?}", r.err);
    assert_eq!(r.err.trim_end().lines().count(), 1, "{what}");
}

#[test]
fn usage_errors_exit_2_and_touch_nothing() {
    let h = Home::new();
    for args in [
        &[][..],
        &["bogus"],
        &["keys"],
        &["keys", "nope"],
        &["login"],                                                                // no user
        &["login", "--user", "@a:b", "--password-stdin", "--password-file", "/x"], // two sources
        &["login", "--user", "@a:b", "--user", "@c:d"],                            // repeated flag
        &["login", "--user", "@a:b", "extra"],                                     // positional
        &["login", "--user", "@a:b", "--bogus"],
        &["logout"],                                                               // no --account
        &["status", "--account"],                                                  // flag without value
        &["keys", "update"],                                                       // no --account
        &["keys", "rotate", "--account", "@a:b"],                                  // no --output
        &["keys", "rotate", "--account", "@a:b", "--output", "relative.key"],
        &["login", "--user", "@a:b", "--password-stdin", "--ca-file", "rel.pem"],
    ] {
        usage_exit(&gnobbi(&h, args, Some(b"x\n"), &[]), &format!("{args:?}"));
    }
    assert!(h.files().is_empty(), "usage errors must not create files: {:?}", h.files());
}

#[test]
fn secrets_in_argv_are_refused_and_never_echoed() {
    let h = Home::new();
    for flag in ["--password", "--recovery-key"] {
        let r = gnobbi(&h, &["login", "--user", "@a:b", flag, SECRET], None, &[]);
        usage_exit(&r, flag);
        assert!(r.err.contains("not accepted as arguments"));
        let r = gnobbi(&h, &["login", "--user", "@a:b", &format!("{flag}={SECRET}")], None, &[]);
        usage_exit(&r, flag);
    }
    let r = gnobbi(&h, &["login", "--user", "@a:b", "--password", SECRET, "--json"], None, &[]);
    assert_eq!(r.code, 2);
    assert_eq!(r.json()["error"]["code"], "usage");
    assert!(h.files().is_empty());
}

#[test]
fn empty_and_malformed_stdin_is_a_usage_error() {
    let h = Home::new();
    let long = vec![b'a'; 4097];
    let ok_len = vec![b'a'; 4096];
    for (what, input) in [("empty", &b""[..]), ("only LF", b"\n"), ("only CRLF", b"\r\n"), ("too long", &long), ("not UTF-8", b"\xff\xfe\n")] {
        usage_exit(&gnobbi(&h, &["login", "--user", "@a:b", "--password-stdin"], Some(input), &[]), what);
    }
    // Exactly 4096 bytes is accepted by the reader: it gets as far as the keyring/server (never exit 2).
    let r = gnobbi(&h, &["login", "--user", "@a:b.invalid", "--password-stdin"], Some(&ok_len), &[]);
    assert_ne!(r.code, 2, "4096 bytes is within the limit: {}", r.err);
    usage_exit(&gnobbi(&h, &["keys", "update", "--account", "@a:b", "--recovery-key-stdin"], Some(b""), &[]), "empty recovery key");
}

#[test]
fn no_terminal_never_prompts() {
    let h = Home::new();
    // No source, stdin closed (/dev/null): must fail at once, not wait.
    let r = gnobbi(&h, &["login", "--user", "@a:b"], None, &[]);
    usage_exit(&r, "login without a source");
    assert!(r.err.contains("no password"));
    let r = gnobbi(&h, &["keys", "update", "--account", "@a:b"], None, &[]);
    usage_exit(&r, "keys update without a source");
    assert!(r.err.contains("no recovery key"));
    // An empty variable is a source, and an empty one.
    usage_exit(&gnobbi(&h, &["login", "--user", "@a:b"], None, &[("GNOBBI_PASSWORD", "")]), "empty GNOBBI_PASSWORD");
    usage_exit(&gnobbi(&h, &["keys", "update", "--account", "@a:b"], None, &[("GNOBBI_RECOVERY_KEY", "")]), "empty GNOBBI_RECOVERY_KEY");
    // Variable plus flag: two sources.
    usage_exit(&gnobbi(&h, &["login", "--user", "@a:b", "--password-stdin"], Some(b"x"), &[("GNOBBI_PASSWORD", SECRET)]), "flag + env");
    assert!(h.files().is_empty());
}

#[test]
fn password_file_edge_cases() {
    let h = Home::new();
    let p = |n: &str| h.0.join(n);
    std::fs::write(p("empty"), "").unwrap();
    std::fs::set_permissions(p("empty"), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::create_dir(p("dir")).unwrap();
    assert!(Command::new("mkfifo").arg(p("fifo")).status().unwrap().success());
    for (what, path) in [("missing", p("missing")), ("empty", p("empty")), ("directory", p("dir")), ("fifo", p("fifo"))] {
        usage_exit(&gnobbi(&h, &["login", "--user", "@a:b", "--password-file", path.to_str().unwrap()], None, &[]), what);
    }
    // World-readable secret file (Docker/systemd secrets): a warning, not an error, and the content stays secret.
    std::fs::write(p("pw"), format!("{SECRET}\n")).unwrap();
    std::fs::set_permissions(p("pw"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let r = gnobbi(&h, &["login", "--user", "@a:b.invalid", "--password-file", p("pw").to_str().unwrap()], None, &[]);
    assert!(r.err.contains("readable by other users"), "{}", r.err);
    assert_ne!(r.code, 2, "a loose file mode is a warning only: {}", r.err);
    // A private one: no warning.
    std::fs::set_permissions(p("pw"), std::fs::Permissions::from_mode(0o600)).unwrap();
    let r = gnobbi(&h, &["login", "--user", "@a:b.invalid", "--password-file", p("pw").to_str().unwrap()], None, &[]);
    assert!(!r.err.contains("readable by other users"), "{}", r.err);
}

#[test]
fn status_and_keys_without_accounts() {
    let h = Home::new();
    let r = gnobbi(&h, &["status"], None, &[]);
    assert_eq!((r.code, r.err.as_str()), (3, "gnobbi: no accounts\n"));
    let r = gnobbi(&h, &["status", "--json"], None, &[]);
    assert_eq!(r.code, 3);
    let j = r.json();
    assert_eq!((&j["ok"], &j["command"], &j["error"]["code"], &j["error"]["exit"]), (&Value::from(false), &Value::from("status"), &Value::from("not_found"), &Value::from(3)));
    assert_eq!(gnobbi(&h, &["status", "--account", "@nobody:x"], None, &[]).code, 3);
    assert_eq!(gnobbi(&h, &["status", "--account", "7"], None, &[]).code, 3);
    assert_eq!(gnobbi(&h, &["keys", "update", "--account", "@nobody:x", "--recovery-key-stdin"], Some(SECRET.as_bytes()), &[]).code, 3);
    assert_eq!(gnobbi(&h, &["keys", "rotate", "--account", "@nobody:x", "--output", "-", "--json"], None, &[]).code, 3);
    // Reads must not create the catalog or its lock.
    assert!(!h.catalog().exists());
}

#[test]
fn logout_without_login_is_a_noop() {
    let h = Home::new();
    let r = gnobbi(&h, &["logout", "--account", "@nobody:x"], None, &[]);
    assert_eq!((r.code, r.out.as_str()), (0, "@nobody:x is not logged in\n"));
    let r = gnobbi(&h, &["logout", "--account", "@nobody:x", "--json"], None, &[]);
    let j = r.json();
    assert_eq!((r.code, &j["ok"], &j["changed"]), (0, &Value::Bool(true), &Value::Bool(false)));
    let r = gnobbi(&h, &["logout", "--account", "3", "--local", "--json"], None, &[]);
    assert_eq!((r.code, r.json()["changed"].clone()), (0, Value::from(false)));
    // With a catalog that exists but does not list the account.
    h.write_catalog(ONE_ACCOUNT);
    let before = std::fs::read(h.catalog()).unwrap();
    assert_eq!(gnobbi(&h, &["logout", "--account", "@other:x"], None, &[]).code, 0);
    assert_eq!(std::fs::read(h.catalog()).unwrap(), before, "a no-op logout must not rewrite the catalog");
}

#[test]
fn damaged_catalog_is_an_error_and_is_never_overwritten() {
    for junk in ["{", "", "[]", r#"{"version":99,"next_id":1,"accounts":[]}"#, r#"{"version":1,"next_id":1,"accounts":[{"id":5,"dir":"x"}]}"#, r#"{"version":1,"next_id":9,"accounts":[{"id":1,"dir":"../../etc"}]}"#] {
        let h = Home::new();
        h.write_catalog(junk);
        for args in [&["status"][..], &["status", "--account", "@a:b"], &["logout", "--account", "@a:b"], &["keys", "rotate", "--account", "@a:b", "--output", "-"]] {
            let r = gnobbi(&h, args, None, &[]);
            assert_eq!(r.code, 9, "{junk:?} {args:?}: {}", r.err);
        }
        let r = gnobbi(&h, &["login", "--user", "@a:b", "--password-stdin", "--json"], Some(b"pw"), &[]);
        // A build without a vault feature stops at exit 8 before it looks at the catalog.
        let no_vault = r.code == 8 && r.json()["error"]["message"].as_str().unwrap().contains("no system keyring support");
        assert!(r.code == 9 || no_vault, "{junk:?}: {} {}", r.code, r.out);
        if r.code == 9 {
            assert_eq!(r.json()["error"]["code"], "storage");
        }
        assert_eq!(std::fs::read_to_string(h.catalog()).unwrap(), junk, "damaged catalog must be left untouched");
    }
}

#[test]
fn unreadable_catalog_is_storage_error() {
    // root ignores file modes: the check is meaningful only for an ordinary user.
    if Command::new("id").arg("-u").output().unwrap().stdout == b"0\n" {
        return;
    }
    let h = Home::new();
    h.write_catalog(ONE_ACCOUNT);
    std::fs::set_permissions(h.catalog(), std::fs::Permissions::from_mode(0o000)).unwrap();
    assert_eq!(gnobbi(&h, &["status"], None, &[]).code, 9);
    std::fs::set_permissions(h.catalog(), std::fs::Permissions::from_mode(0o600)).unwrap();
    // Read-only config directory: a catalog write fails cleanly with exit 9 (login needs the write).
    std::fs::set_permissions(h.catalog().parent().unwrap(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let r = gnobbi(&h, &["login", "--user", "@b:example.org", "--password-stdin"], Some(b"pw"), &[]);
    std::fs::set_permissions(h.catalog().parent().unwrap(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(r.code, 9, "{}", r.err);
}

#[test]
fn no_home_is_a_storage_error() {
    let o = Command::new(env!("CARGO_BIN_EXE_gnobbi")).args(["account", "status", "--json"]).env_clear().stdin(Stdio::null()).output().unwrap();
    assert_eq!(o.status.code(), Some(9));
    let j: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(j["error"]["code"], "storage");
}

#[test]
fn status_of_a_catalog_entry_without_a_profile_is_a_clean_nonzero_json() {
    let h = Home::new();
    h.write_catalog(ONE_ACCOUNT);
    let r = gnobbi(&h, &["status", "--json"], None, &[]);
    assert_ne!(r.code, 0, "an account with no saved session is not valid");
    let j = r.json();
    let row = &j["accounts"][0];
    assert_eq!((&j["ok"], &j["exit"], &row["user_id"]), (&Value::from(false), &Value::from(r.code), &Value::from("@a:example.org")));
    assert!(["expired", "broken", "keyring_unavailable"].contains(&row["session"].as_str().unwrap()), "{row}");
    assert!(row["verified"].is_null());
    let text = j.to_string();
    assert!(!text.contains("syt_") && !text.contains("secret"), "{text}");
}

#[test]
fn rotate_never_overwrites_the_output_and_leaves_no_temp_file() {
    let h = Home::new();
    let out = h.0.join("recovery.key");
    std::fs::write(&out, "precious").unwrap();
    let r = gnobbi(&h, &["keys", "rotate", "--account", "@a:b", "--output", out.to_str().unwrap()], None, &[]);
    usage_exit(&r, "existing --output");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "precious");
    // A dangling symlink counts as existing too (never followed, never replaced).
    let link = h.0.join("link.key");
    std::os::unix::fs::symlink(h.0.join("nowhere"), &link).unwrap();
    usage_exit(&gnobbi(&h, &["keys", "rotate", "--account", "@a:b", "--output", link.to_str().unwrap()], None, &[]), "symlink --output");
    assert!(!h.0.join("nowhere").exists());
    // Unknown account: exit 3 and the temp file created up front is gone again.
    let fresh = h.0.join("fresh.key");
    let r = gnobbi(&h, &["keys", "rotate", "--account", "@nobody:x", "--output", fresh.to_str().unwrap()], None, &[]);
    assert_eq!(r.code, 3);
    let left: Vec<_> = h.files().into_iter().filter(|f| f.file_name().is_some_and(|n| n.to_string_lossy().contains("fresh.key"))).collect();
    assert!(left.is_empty(), "leftover: {left:?}");
    // Output folder that does not exist: storage error before the server is touched.
    let r = gnobbi(&h, &["keys", "rotate", "--account", "@a:b", "--output", h.0.join("no/such/dir/k").to_str().unwrap()], None, &[]);
    assert_eq!(r.code, 9, "{}", r.err);
}

#[test]
fn json_is_one_object_on_stdout_for_every_failure() {
    let h = Home::new();
    for (args, exit, code) in [
        (&["login", "--json"][..], 2, "usage"),
        (&["bogus", "--json"], 2, "usage"),
        (&["status", "--account", "@x:y", "--json"], 3, "not_found"),
        (&["keys", "update", "--account", "@x:y", "--recovery-key-stdin", "--json"], 3, "not_found"),
    ] {
        let r = gnobbi(&h, args, Some(SECRET.as_bytes()), &[]);
        let j = r.json();
        assert_eq!((r.code, &j["ok"], &j["error"]["exit"], &j["error"]["code"]), (exit, &Value::from(false), &Value::from(exit), &Value::from(code)), "{args:?}");
        assert!(j["error"]["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
}

#[test]
fn ca_file_problems_are_usage_errors() {
    let h = Home::new();
    let r = gnobbi(&h, &["status", "--ca-file", h.0.join("missing.pem").to_str().unwrap()], None, &[]);
    usage_exit(&r, "missing CA file");
    // A file that is not PEM must not be accepted as a trust root.
    std::fs::write(h.0.join("junk.pem"), "not a certificate").unwrap();
    let r = gnobbi(&h, &["login", "--user", "@a:b.invalid", "--password-stdin", "--ca-file", h.0.join("junk.pem").to_str().unwrap()], Some(b"pw"), &[]);
    assert_ne!(r.code, 0, "{}", r.out);
}
