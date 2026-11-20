use super::*;
use matrix_engine::signin::VaultError;

fn args(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}
fn no_env(_: &str) -> Option<String> {
    None
}
fn run<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
}
fn temp_dirs(name: &str) -> AppDirs {
    let base = std::env::temp_dir().join(format!("gnobbi-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    AppDirs { config: base.join("config"), data: base.join("data"), cache: base.join("cache") }
}

#[test]
fn parse_login_flags_and_env() {
    let p = parse(&args("login --user @a:b.c --password-stdin --json"), &no_env).unwrap();
    assert_eq!(p.cmd, Cmd::Login { user: "@a:b.c".into(), homeserver: None, source: SecretSource::Stdin });
    assert!(p.json);
    let p = parse(&args("login --user=@a:b --homeserver h.example --password-file /p"), &no_env).unwrap();
    assert_eq!(p.cmd, Cmd::Login { user: "@a:b".into(), homeserver: Some("h.example".into()), source: SecretSource::File("/p".into()) });
    let env = |k: &str| match k {
        "GNOBBI_USER" => Some("@e:s".to_owned()),
        "GNOBBI_PASSWORD" => Some(String::new()),
        _ => None,
    };
    let p = parse(&args("login"), &env).unwrap();
    assert_eq!(p.cmd, Cmd::Login { user: "@e:s".into(), homeserver: None, source: SecretSource::Env });
    // No source at all: ask (or fail without a terminal).
    let p = parse(&args("login --user @a:b"), &no_env).unwrap();
    assert!(matches!(p.cmd, Cmd::Login { source: SecretSource::Ask, .. }));
}

#[test]
fn parse_rejects_what_the_contract_forbids() {
    let env_pw = |k: &str| (k == "GNOBBI_PASSWORD").then(String::new);
    for (line, env, why) in [
        ("login --user @a:b --password hunter2", false, "secret in argv"),
        ("login --user @a:b --password=hunter2", false, "secret in argv, = form"),
        ("login --user @a:b --recovery-key x", false, "recovery key in argv"),
        ("login --user @a:b --password-stdin --password-file /p", false, "two sources"),
        ("login --user @a:b --password-stdin", true, "flag plus environment"),
        ("login --password-stdin", false, "no user"),
        ("login --user @a:b --user @c:d --password-stdin", false, "repeated flag"),
        ("login --user @a:b --password-stdin extra", false, "positional"),
        ("login --user @a:b --password-stdin --bogus", false, "unknown flag"),
        ("login --user", false, "missing value"),
        ("logout", false, "no account"),
        ("logout --account @a:b --password-stdin", false, "flag of another command"),
        ("status --local", false, "flag of another command"),
        ("frobnicate", false, "unknown command"),
        ("keys", false, "keys without verb"),
        ("", false, "nothing"),
    ] {
        let env: &dyn Fn(&str) -> Option<String> = if env { &env_pw } else { &no_env };
        assert!(parse(&args(line), env).is_err(), "{why}: {line}");
    }
    assert_eq!(parse(&args("login --user @a:b --password x"), &no_env).unwrap_err(), SECRET_IN_ARGV);
}

#[test]
fn parse_logout_status_keys() {
    let p = parse(&args("logout --account @a:b --local"), &no_env).unwrap();
    assert_eq!(p.cmd, Cmd::Logout { account: "@a:b".into(), local: true });
    let env = |k: &str| (k == "GNOBBI_ACCOUNT").then(|| "3".to_owned());
    assert_eq!(parse(&args("status"), &env).unwrap().cmd, Cmd::Status { account: Some("3".into()) });
    assert_eq!(parse(&args("status --ca-file /x.pem"), &no_env).unwrap().ca_file, Some("/x.pem".into()));
    assert_eq!(parse(&args("keys rotate --json"), &no_env).unwrap().cmd, Cmd::Keys);
}

fn secret(bytes: &[u8]) -> Result<String, i32> {
    read_limited(bytes).map(|s| s.to_string()).map_err(|e| e.exit)
}

#[test]
fn secrets_strip_one_newline_and_respect_limits() {
    assert_eq!(secret(b"pw\n").unwrap(), "pw");
    assert_eq!(secret(b"pw\r\n").unwrap(), "pw");
    assert_eq!(secret(b"pw\n\n").unwrap(), "pw\n");
    assert_eq!(secret(b" pw \t").unwrap(), " pw \t", "nothing else is trimmed");
    assert_eq!(secret(b""), Err(2));
    assert_eq!(secret(b"\n"), Err(2));
    assert_eq!(secret(&[b'x'; 4096]).unwrap().len(), 4096);
    assert_eq!(secret(&[b'x'; 4097]), Err(2));
    assert_eq!(secret(&[0xff, 0xfe]), Err(2));
}

#[test]
fn secret_file_must_be_a_regular_file() {
    let dir = temp_dirs("secretfile").config;
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("pw");
    std::fs::write(&file, "from file\n").unwrap();
    assert_eq!(read_secret_file(&file).unwrap().as_str(), "from file");
    assert_eq!(read_secret_file(&dir).unwrap_err().exit, 2);
    assert_eq!(read_secret_file(&dir.join("missing")).unwrap_err().exit, 2);
}

#[test]
fn error_table_exit_codes() {
    use AuthError as A;
    use SignInError as S;
    let rows: [(CliError, i32, &str); 20] = [
        (signin_error(S::InvalidHomeserver), 2, "usage"),
        (signin_error(S::InvalidUserId), 2, "usage"),
        (signin_error(S::InvalidSecret), 2, "usage"),
        (signin_error(S::InvalidTrustRoot), 2, "usage"),
        (signin_error(S::Discovery), 6, "temporary"),
        (signin_error(S::UnsupportedLogin), 7, "server_unsupported"),
        (signin_error(S::NoSlidingSync), 7, "server_unsupported"),
        (signin_error(S::ServerStatus), 7, "server_unsupported"),
        (signin_error(S::Vault(VaultError::Locked)), 8, "keyring_unavailable"),
        (signin_error(S::Storage), 9, "storage"),
        (signin_error(S::AlreadySignedIn), 9, "storage"),
        (signin_error(S::ProtectionMismatch), 9, "storage"),
        (signin_error(S::ProfileBusy), 10, "busy"),
        (signin_error(S::NoSession), 1, "internal"),
        (auth_error(A::CredentialsRejected), 4, "credentials_rejected"),
        (auth_error(A::LoginFailed), 4, "credentials_rejected"),
        (auth_error(A::SessionExpired), 5, "session_expired"),
        (signin_error(S::Authentication(A::LoggedOut)), 5, "session_expired"),
        (auth_error(A::Transport), 6, "temporary"),
        (auth_error(A::Storage), 9, "storage"),
    ];
    for (error, exit, code) in rows {
        assert_eq!((error.exit, error.code), (exit, code), "{error:?}");
    }
    assert_eq!(State::Valid.exit(), 0);
    assert_eq!(State::of_signin(S::ProfileBusy), State::InUse);
    assert_eq!(State::of_signin(S::Authentication(A::Transport)), State::Unverified);
    assert_eq!(State::Unverified.exit(), 6);
}

#[test]
fn catalog_update_keeps_concurrent_changes() {
    let dirs = temp_dirs("update");
    let threads: Vec<_> = (0..8)
        .map(|n| {
            let dirs = dirs.clone();
            std::thread::spawn(move || AccountCatalog::update(&dirs, |c| c.add(&format!("@u{n}:s"), "s", "l")).unwrap())
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(AccountCatalog::load(&dirs).accounts().len(), 8);
}

#[test]
fn catalog_update_refuses_a_damaged_catalog() {
    let dirs = temp_dirs("damaged");
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("accounts.json"), "not json").unwrap();
    let mut ran = false;
    assert!(AccountCatalog::update(&dirs, |_| ran = true).is_err());
    assert!(!ran);
    assert_eq!(std::fs::read(dirs.config.join("accounts.json")).unwrap(), b"not json");
    assert_eq!(run(status(&dirs, None, None)).err().map(|e| e.exit), Some(9));
}

#[test]
fn status_without_accounts_or_with_unknown_account_is_not_found() {
    let dirs = temp_dirs("status-empty");
    assert_eq!(run(status(&dirs, None, None)).err().map(|e| e.exit), Some(3));
    AccountCatalog::update(&dirs, |c| c.add("@a:s", "s", "a")).unwrap();
    assert_eq!(run(status(&dirs, Some("@x:s"), None)).err().map(|e| e.exit), Some(3));
    assert_eq!(run(status(&dirs, Some("99"), None)).err().map(|e| e.exit), Some(3));
}

#[test]
fn status_of_an_account_without_session_is_expired_not_a_crash() {
    let dirs = temp_dirs("status-nosession");
    let id = AccountCatalog::update(&dirs, |c| c.add("@a:s", "s", "a")).unwrap();
    let out = run(status(&dirs, Some("@a:s"), None)).unwrap();
    assert_eq!(out.exit, 5);
    assert_eq!(out.json["accounts"][0]["session"], "expired");
    assert_eq!(out.json["accounts"][0]["id"], id.as_u64());
    assert_eq!(out.json["accounts"][0]["verified"], Value::Null);
    assert_eq!(out.json["ok"], false);
}

#[test]
fn logout_is_idempotent_and_local_removes_everything() {
    let dirs = temp_dirs("logout");
    // Unknown account: success, nothing changed.
    let out = run(logout(&dirs, "@a:s", false, None)).unwrap();
    assert_eq!((out.exit, &out.json["changed"]), (0, &Value::Bool(false)));
    let id = AccountCatalog::update(&dirs, |c| c.add("@a:s", "s", "a")).unwrap();
    let catalog = AccountCatalog::load(&dirs);
    catalog.ensure_dirs(&dirs, id).unwrap();
    let data = catalog.data_dir(&dirs, id).unwrap();
    std::fs::create_dir_all(data.join("account-1")).unwrap();
    std::fs::write(data.join("account-1").join("x"), "x").unwrap();
    let cache = catalog.cache_dir(&dirs, id).unwrap();

    let out = run(logout(&dirs, "@a:s", true, None)).unwrap();
    assert_eq!((out.exit, &out.json["changed"], &out.json["remote_revoked"]), (0, &Value::Bool(true), &Value::Bool(false)));
    assert!(!data.exists() && !cache.exists());
    assert!(AccountCatalog::load(&dirs).accounts().is_empty());
    // Second run: already gone.
    let again = run(logout(&dirs, "@a:s", true, None)).unwrap();
    assert_eq!((again.exit, &again.json["changed"]), (0, &Value::Bool(false)));
}

#[test]
fn logout_resumes_after_a_crash_between_steps() {
    // Folders already deleted, entry still listed: the rerun finishes the job.
    let dirs = temp_dirs("logout-resume");
    let id = AccountCatalog::update(&dirs, |c| c.add("@a:s", "s", "a")).unwrap();
    let out = run(logout(&dirs, &id.as_u64().to_string(), true, None)).unwrap();
    assert_eq!(out.json["changed"], true);
    assert!(AccountCatalog::load(&dirs).accounts().is_empty());
}
