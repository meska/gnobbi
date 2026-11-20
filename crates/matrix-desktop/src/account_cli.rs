//! `gnobbi account login|logout|status`: accounts from the command line, with no GUI.
//! Same catalog, profiles and OS keyring as the app. Secrets come only from stdin, a file, the
//! environment or a no-echo prompt (never argv), and are never printed. Exit codes and JSON
//! are a contract for scripts: the ACCOUNT_CLI notes §3 and §5.
use crate::{AccountCatalog, AppDirs, Platform, app_dirs};
use matrix_domain::AccountId;
use matrix_engine::auth::{AccountSession, AuthError};
use matrix_engine::signin::{Secret, SignInError, SignInProfile, forget_secrets, platform_source};
use serde_json::{Value, json};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

const MAX_SECRET: usize = 4096;
const MAX_CA: u64 = 1 << 20;
const VAULT_WAIT: Duration = Duration::from_secs(10);
const WHOAMI_WAIT: Duration = Duration::from_secs(3);
const SECRET_IN_ARGV: &str = "secrets are not accepted as arguments; use --password-stdin";

#[derive(Debug, PartialEq, Eq)]
pub enum SecretSource {
    Stdin,
    File(PathBuf),
    Env,
    /// No flag, no variable: prompt if stdin is a terminal, otherwise a usage error.
    Ask,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
    Login { user: String, homeserver: Option<String>, source: SecretSource },
    Logout { account: String, local: bool },
    Status { account: Option<String> },
    /// Recognised, implemented by the keys card.
    Keys,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Parsed {
    pub cmd: Cmd,
    pub json: bool,
    pub ca_file: Option<PathBuf>,
}

/// One flag per name; `spec` is (name, takes a value).
fn flags(args: &[String], spec: &[(&str, bool)]) -> Result<Vec<(String, Option<String>)>, String> {
    let mut out: Vec<(String, Option<String>)> = vec![];
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_owned())),
            _ => (arg.as_str(), None),
        };
        if matches!(name, "--password" | "--recovery-key") {
            return Err(SECRET_IN_ARGV.into());
        }
        let Some(&(_, takes)) = spec.iter().find(|(n, _)| *n == name) else {
            return Err(format!("unknown argument {name}"));
        };
        if out.iter().any(|(n, _)| n == name) {
            return Err(format!("{name} given twice"));
        }
        let value = match (takes, inline) {
            (true, Some(v)) => Some(v),
            (true, None) => Some(it.next().cloned().ok_or(format!("{name} needs a value"))?),
            (false, None) => None,
            (false, Some(_)) => return Err(format!("{name} takes no value")),
        };
        out.push((name.to_owned(), value));
    }
    Ok(out)
}

/// `env` is how the process environment is read (the secrets only report presence).
pub fn parse(args: &[String], env: &dyn Fn(&str) -> Option<String>) -> Result<Parsed, String> {
    let Some((sub, rest)) = args.split_first() else {
        return Err("usage: gnobbi account login|logout|status|keys".into());
    };
    let (sub, rest) = match (sub.as_str(), rest.split_first()) {
        ("keys", Some((k, r))) if matches!(k.as_str(), "update" | "rotate") => ("keys", r),
        ("keys", _) => return Err("usage: gnobbi account keys update|rotate".into()),
        (s @ ("login" | "logout" | "status"), _) => (s, rest),
        (other, _) => return Err(format!("unknown command {other}")),
    };
    let common = [("--json", false), ("--ca-file", true)];
    let spec: Vec<(&str, bool)> = match sub {
        "login" => [&common[..], &[("--user", true), ("--homeserver", true), ("--password-stdin", false), ("--password-file", true)]].concat(),
        "logout" => [&common[..], &[("--account", true), ("--local", false)]].concat(),
        "status" => [&common[..], &[("--account", true)]].concat(),
        _ => return Ok(Parsed { cmd: Cmd::Keys, json: rest.iter().any(|a| a == "--json"), ca_file: None }),
    };
    let found = flags(rest, &spec)?;
    let get = |name: &str| found.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
    let value = |name: &str, var: &str| get(name).flatten().or_else(|| env(var).filter(|v| !v.is_empty()));
    let ca_file = get("--ca-file").flatten().map(PathBuf::from);
    let json = get("--json").is_some();
    let cmd = match sub {
        "login" => {
            let user = value("--user", "GNOBBI_USER").ok_or("--user is required (or GNOBBI_USER)")?;
            let sources = [
                get("--password-stdin").map(|_| SecretSource::Stdin),
                get("--password-file").flatten().map(|p| SecretSource::File(p.into())),
                env("GNOBBI_PASSWORD").map(|_| SecretSource::Env),
            ];
            let mut sources = sources.into_iter().flatten();
            let source = sources.next().unwrap_or(SecretSource::Ask);
            if sources.next().is_some() {
                return Err("give the password from one source only".into());
            }
            Cmd::Login { user, homeserver: value("--homeserver", "GNOBBI_HOMESERVER"), source }
        }
        "logout" => Cmd::Logout {
            account: value("--account", "GNOBBI_ACCOUNT").ok_or("--account is required (or GNOBBI_ACCOUNT)")?,
            local: get("--local").is_some(),
        },
        _ => Cmd::Status { account: value("--account", "GNOBBI_ACCOUNT") },
    };
    Ok(Parsed { cmd, json, ca_file })
}

/// Failure with its contract: process exit code, stable JSON code, one fixed line (no secrets, no server text).
#[derive(Debug, PartialEq, Eq)]
pub struct CliError {
    pub exit: i32,
    pub code: &'static str,
    pub message: String,
}

fn fail(exit: i32, code: &'static str, message: impl Into<String>) -> CliError {
    CliError { exit, code, message: message.into() }
}
fn usage(message: impl Into<String>) -> CliError {
    fail(2, "usage", message)
}
fn storage(message: &str) -> CliError {
    fail(9, "storage", message)
}

pub fn auth_error(e: AuthError) -> CliError {
    match e {
        AuthError::InvalidUserId => usage("invalid user id"),
        AuthError::CredentialsRejected | AuthError::LoginFailed | AuthError::IdentityMismatch => {
            fail(4, "credentials_rejected", "credentials rejected; do not retry")
        }
        AuthError::SessionExpired | AuthError::LoggedOut => fail(5, "session_expired", "session expired; log in again"),
        AuthError::Transport | AuthError::RateLimited => fail(6, "temporary", "homeserver unreachable or busy; retry later"),
        AuthError::Storage => storage("account storage error"),
    }
}

pub fn signin_error(e: SignInError) -> CliError {
    match e {
        SignInError::InvalidHomeserver => usage("invalid homeserver"),
        SignInError::InvalidUserId => usage("invalid user id"),
        SignInError::InvalidSecret => usage("invalid secret (empty or longer than 4096 bytes)"),
        SignInError::InvalidTrustRoot => usage("invalid CA file"),
        SignInError::InvalidStorePath => usage("invalid store path"),
        SignInError::Discovery => fail(6, "temporary", "homeserver discovery failed; retry later"),
        SignInError::UnsupportedLogin | SignInError::NoSlidingSync | SignInError::ServerStatus => {
            fail(7, "server_unsupported", "homeserver not supported")
        }
        SignInError::Vault(_) => fail(8, "keyring_unavailable", "system keyring unavailable or locked"),
        SignInError::Storage | SignInError::AlreadySignedIn | SignInError::AttemptLimit | SignInError::ProtectionMismatch => {
            storage("account storage error")
        }
        SignInError::ProfileBusy => fail(10, "busy", "account is open elsewhere; close Gnobbi and retry"),
        SignInError::NoSession => fail(1, "internal", "no saved session"),
        SignInError::Authentication(a) => auth_error(a),
    }
}

fn io_error(_: std::io::Error) -> CliError {
    storage("cannot update the account catalog")
}

/// Reads a secret from stdin or a file: at most 4096 bytes, one trailing `\n`/`\r\n` removed, never empty.
pub fn read_limited(reader: impl Read) -> Result<Zeroizing<String>, CliError> {
    let mut bytes = Zeroizing::new(Vec::new());
    reader.take(MAX_SECRET as u64 + 1).read_to_end(&mut bytes).map_err(|_| usage("cannot read the secret"))?;
    if bytes.len() > MAX_SECRET {
        return Err(usage("secret longer than 4096 bytes"));
    }
    let end = bytes.strip_suffix(b"\n").map(|b| b.strip_suffix(b"\r").unwrap_or(b)).unwrap_or(&bytes).len();
    bytes.truncate(end);
    if bytes.is_empty() {
        return Err(usage("secret is empty"));
    }
    String::from_utf8(std::mem::take(&mut *bytes)).map(Zeroizing::new).map_err(|_| usage("secret is not UTF-8"))
}

fn read_secret_file(path: &Path) -> Result<Zeroizing<String>, CliError> {
    let meta = std::fs::metadata(path).map_err(|_| usage("cannot read the password file"))?;
    if !meta.is_file() {
        return Err(usage("the password file is not a regular file"));
    }
    #[cfg(unix)]
    if std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o077 != 0 {
        eprintln!("gnobbi: warning: the password file is readable by other users");
    }
    read_limited(std::fs::File::open(path).map_err(|_| usage("cannot read the password file"))?)
}

fn read_password(source: &SecretSource, env_secret: Option<Zeroizing<String>>) -> Result<Zeroizing<String>, CliError> {
    match source {
        SecretSource::Stdin => read_limited(std::io::stdin().lock()),
        SecretSource::File(path) => read_secret_file(path),
        SecretSource::Env => env_secret.filter(|v| !v.is_empty()).ok_or_else(|| usage("GNOBBI_PASSWORD is empty")),
        SecretSource::Ask if std::io::stdin().is_terminal() => {
            let typed = rpassword::prompt_password("Password: ").map_err(|_| usage("cannot read the password"))?;
            let typed = Zeroizing::new(typed);
            if typed.is_empty() { Err(usage("secret is empty")) } else { Ok(typed) }
        }
        SecretSource::Ask => Err(usage("no password: use --password-stdin")),
    }
}

fn read_ca(path: &Path) -> Result<Vec<u8>, CliError> {
    if !path.is_absolute() {
        return Err(usage("--ca-file must be an absolute path"));
    }
    let mut pem = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_CA).read_to_end(&mut pem))
        .map_err(|_| usage("cannot read the CA file"))?;
    Ok(pem)
}

/// Session state shown by `status` and decided by `login` for an account already listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Valid,
    Unverified,
    Expired,
    InUse,
    KeyringUnavailable,
    Broken,
}
impl State {
    fn name(self) -> &'static str {
        match self {
            State::Valid => "valid",
            State::Unverified => "unverified",
            State::Expired => "expired",
            State::InUse => "in_use",
            State::KeyringUnavailable => "keyring_unavailable",
            State::Broken => "broken",
        }
    }
    pub fn exit(self) -> i32 {
        match self {
            State::Valid => 0,
            State::Expired => 5,
            State::Unverified => 6,
            State::KeyringUnavailable => 8,
            State::Broken => 9,
            State::InUse => 10,
        }
    }
    fn of_signin(e: SignInError) -> State {
        match e {
            SignInError::ProfileBusy => State::InUse,
            SignInError::Vault(_) => State::KeyringUnavailable,
            SignInError::Authentication(a) => State::of_auth(a),
            _ => State::Broken,
        }
    }
    fn of_auth(e: AuthError) -> State {
        match e {
            AuthError::SessionExpired | AuthError::LoggedOut => State::Expired,
            AuthError::Transport | AuthError::RateLimited => State::Unverified,
            _ => State::Broken,
        }
    }
    fn into_error(self) -> CliError {
        let (code, message) = match self {
            State::InUse => ("busy", "account is open elsewhere; close Gnobbi and retry"),
            State::KeyringUnavailable => ("keyring_unavailable", "system keyring unavailable or locked"),
            _ => ("storage", "account storage error"),
        };
        fail(self.exit(), code, message)
    }
}

/// Opens the saved session of the profile at `root`. `None`: no saved session (nothing to restore).
async fn restore(root: PathBuf, ca: Option<Vec<u8>>) -> Result<Option<AccountSession>, SignInError> {
    if !root.exists() {
        return Ok(None);
    }
    let mut profile = SignInProfile::open(root.clone())?;
    let Some(active) = profile.active_account() else { return Ok(None) };
    if !root.join(format!("account-{}", active.as_u64())).join("session.sealed").exists() {
        return Ok(None);
    }
    let source: std::sync::Arc<_> = platform_source().ok_or(SignInError::Vault(matrix_engine::signin::VaultError::Unavailable))?.into();
    profile.unlock_vault_timed(source, ca, VAULT_WAIT).await.map(Some)
}

struct Checked {
    state: State,
    user: Option<String>,
    device: Option<String>,
    verified: Option<bool>,
}

/// Restore + `whoami` (3 s): the real state of the saved session. Read-only, no sync.
async fn check(dirs: &AppDirs, catalog: &AccountCatalog, id: AccountId, ca: Option<Vec<u8>>) -> Checked {
    let bad = |state| Checked { state, user: None, device: None, verified: None };
    let Some(root) = catalog.data_dir(dirs, id) else { return bad(State::Broken) };
    let session = match restore(root, ca).await {
        Ok(Some(s)) => s,
        Ok(None) => return bad(State::Expired),
        Err(e) => return bad(State::of_signin(e)),
    };
    match tokio::time::timeout(WHOAMI_WAIT, session.whoami()).await {
        Ok(Ok(who)) => Checked {
            state: State::Valid,
            user: Some(who.user_id().to_owned()),
            device: Some(who.device_id().to_owned()),
            verified: Some(session.keys_recovered().await),
        },
        Ok(Err(e)) => bad(State::of_auth(e)),
        Err(_) => bad(State::Unverified),
    }
}

pub struct Outcome {
    json: Value,
    text: String,
    exit: i32,
}

fn account_json(catalog: &AccountCatalog, id: AccountId, user: Option<&str>, device: Option<&str>) -> Value {
    let entry = catalog.get(id);
    json!({
        "id": id.as_u64(),
        "user_id": user.map(str::to_owned).or_else(|| entry.and_then(|e| e.user_id.clone())),
        "homeserver": entry.and_then(|e| e.homeserver.clone()),
        "device_id": device,
    })
}

/// A matrix ID or the numeric catalog ID.
fn select(catalog: &AccountCatalog, selector: &str) -> Option<AccountId> {
    match selector.parse::<u64>() {
        Ok(n) => catalog.get(AccountId::new(n)).map(|a| a.id),
        Err(_) => catalog.id_for_user(selector),
    }
}

fn load_catalog(dirs: &AppDirs) -> Result<AccountCatalog, CliError> {
    let catalog = AccountCatalog::load(dirs);
    if catalog.is_damaged() { Err(storage("the account catalog is damaged")) } else { Ok(catalog) }
}

async fn status(dirs: &AppDirs, account: Option<&str>, ca: Option<Vec<u8>>) -> Result<Outcome, CliError> {
    let catalog = load_catalog(dirs)?;
    let ids: Vec<AccountId> = match account {
        Some(sel) => vec![select(&catalog, sel).ok_or_else(|| fail(3, "not_found", "no such account"))?],
        None if catalog.accounts().is_empty() => return Err(fail(3, "not_found", "no accounts")),
        None => catalog.accounts().iter().map(|a| a.id).collect(),
    };
    let (mut rows, mut lines, mut exit) = (vec![], vec![], 0);
    for id in ids {
        let c = check(dirs, &catalog, id, ca.clone()).await;
        if exit == 0 {
            exit = c.state.exit();
        }
        let mut row = account_json(&catalog, id, c.user.as_deref(), c.device.as_deref());
        row["session"] = c.state.name().into();
        row["verified"] = c.verified.map_or(Value::Null, Value::Bool);
        let who = row["user_id"].as_str().map_or_else(|| format!("account {}", id.as_u64()), str::to_owned);
        let keys = match c.verified {
            Some(true) => " verified",
            Some(false) => " not-verified",
            None => "",
        };
        lines.push(format!("{who} {}{keys}", c.state.name()));
        rows.push(row);
    }
    let mut json = json!({ "ok": exit == 0, "command": "status", "accounts": rows });
    if exit != 0 {
        json["exit"] = exit.into();
    }
    Ok(Outcome { json, text: lines.join("\n"), exit })
}

/// Logout steps 4-6: dirs, keyring items, catalog entry last (a crash leaves it re-runnable).
fn forget_local(dirs: &AppDirs, id: AccountId, stores: &[PathBuf]) -> Result<(), CliError> {
    let catalog = load_catalog(dirs)?;
    for dir in [catalog.data_dir(dirs, id), catalog.cache_dir(dirs, id)].into_iter().flatten() {
        match std::fs::remove_dir_all(dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(storage("cannot remove the account folders")),
            _ => {}
        }
    }
    forget_secrets(id, stores).map_err(|_| fail(8, "keyring_unavailable", "cannot remove the keyring items"))?;
    AccountCatalog::update(dirs, |c| c.remove(id)).map(drop).map_err(io_error)
}

fn stores_of(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return vec![] };
    entries.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("account-")).map(|e| e.path()).collect()
}

async fn logout(dirs: &AppDirs, selector: &str, local: bool, ca: Option<Vec<u8>>) -> Result<Outcome, CliError> {
    let catalog = load_catalog(dirs)?;
    let Some(id) = select(&catalog, selector) else {
        let json = json!({ "ok": true, "command": "logout", "changed": false, "account": { "user_id": selector } });
        return Ok(Outcome { json, text: format!("{selector} is not logged in"), exit: 0 });
    };
    let user = catalog.get(id).and_then(|e| e.user_id.clone());
    let root = catalog.data_dir(dirs, id).ok_or_else(|| fail(1, "internal", "account folder"))?;
    let stores = stores_of(&root);
    let mut revoked = false;
    if !local {
        match restore(root, ca).await {
            Ok(Some(mut session)) => match session.logout().await {
                Ok(()) | Err(AuthError::SessionExpired | AuthError::LoggedOut) => revoked = true,
                Err(e) => return Err(auth_error(e)),
            },
            Ok(None) => {}
            Err(SignInError::Authentication(AuthError::SessionExpired | AuthError::LoggedOut)) => revoked = true,
            Err(e @ SignInError::Vault(_)) => {
                let mut error = signin_error(e);
                error.message.push_str("; use --local to skip revoking the device");
                return Err(error);
            }
            Err(e) => return Err(signin_error(e)),
        }
    }
    forget_local(dirs, id, &stores)?;
    let label = user.clone().unwrap_or_else(|| format!("account {}", id.as_u64()));
    let json = json!({ "ok": true, "command": "logout", "changed": true, "remote_revoked": revoked,
        "account": { "id": id.as_u64(), "user_id": user } });
    Ok(Outcome { json, text: format!("logged out {label}"), exit: 0 })
}

async fn login(dirs: &AppDirs, user: String, homeserver: Option<String>, mut password: Zeroizing<String>, ca: Option<Vec<u8>>) -> Result<Outcome, CliError> {
    let source = platform_source().ok_or_else(|| fail(8, "keyring_unavailable", "this build has no system keyring support"))?;
    let (existing, root) = AccountCatalog::update(dirs, |c| {
        c.migrate_legacy(dirs);
        (c.id_for_user(&user), c.next_root(dirs))
    })
    .map_err(io_error)?;
    if let Some(id) = existing {
        let catalog = load_catalog(dirs)?;
        let c = check(dirs, &catalog, id, ca.clone()).await;
        match c.state {
            State::Valid | State::Unverified => {
                let json = json!({ "ok": true, "command": "login", "changed": false,
                    "account": account_json(&catalog, id, Some(&user), c.device.as_deref()) });
                return Ok(Outcome { json, text: format!("already logged in {user}"), exit: 0 });
            }
            // Revoked or lost: clean it and sign in again below.
            State::Expired => {
                let stores = catalog.data_dir(dirs, id).map(|r| stores_of(&r)).unwrap_or_default();
                forget_local(dirs, id, &stores)?;
            }
            other => return Err(other.into_error()),
        }
    }
    let secret = Secret::new(std::mem::take(&mut *password)).map_err(signin_error)?;
    let homeserver = homeserver.or_else(|| user.split_once(':').map(|(_, server)| server.to_owned())).unwrap_or_default();
    #[cfg(unix)]
    crate::create_private_dir(&root).map_err(|_| storage("cannot create the account folder"))?;
    #[cfg(not(unix))]
    std::fs::create_dir_all(&root).map_err(|_| storage("cannot create the account folder"))?;
    let rid = AccountId::new(root.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse().ok()).unwrap_or(0));
    let result = match SignInProfile::open(root.clone()) {
        Ok(mut profile) => profile
            .password_login_vault(&*source, homeserver.clone(), user.clone(), secret, ca)
            .await
            .map_err(signin_error),
        Err(e) => Err(signin_error(e)),
    };
    let mut session = match result {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&root);
            return Err(e);
        }
    };
    let added = AccountCatalog::update(dirs, |c| {
        (c.next_root(dirs) == root).then(|| {
            let id = c.add(&user, &homeserver, &user);
            (id, c.ensure_dirs(dirs, id))
        })
    });
    let error = match added {
        Ok(Some((id, Ok(())))) => {
            let device = session.whoami().await.ok().map(|w| w.device_id().to_owned());
            let catalog = AccountCatalog::load(dirs);
            let json = json!({ "ok": true, "command": "login", "changed": true,
                "account": account_json(&catalog, id, Some(&user), device.as_deref()) });
            let text = format!("logged in {user} (account {}{})", id.as_u64(), device.map(|d| format!(", device {d}")).unwrap_or_default());
            return Ok(Outcome { json, text, exit: 0 });
        }
        Ok(None) => fail(10, "busy", "another process added an account meanwhile; retry"),
        Ok(Some((_, Err(e)))) | Err(e) => io_error(e),
    };
    // Roll back: revoke the new device, drop its folder, keyring items and any catalog entry.
    let revoked = session.logout().await.is_ok();
    drop(session);
    let stores = stores_of(&root);
    let _ = std::fs::remove_dir_all(&root);
    let _ = forget_secrets(rid, &stores);
    let _ = AccountCatalog::update(dirs, |c| {
        if let Some(id) = c.id_for_user(&user).filter(|id| c.data_dir(dirs, *id).as_deref() == Some(root.as_path())) {
            c.remove(id);
        }
    });
    Err(if revoked { error } else { CliError { message: format!("{}; the new device may still be listed on the server", error.message), ..error } })
}

fn take_env(key: &str) -> Option<Zeroizing<String>> {
    let value = std::env::var(key).ok()?;
    // SAFETY: called first thing in `main`, before the runtime or any thread exists.
    unsafe { std::env::remove_var(key) };
    Some(Zeroizing::new(value))
}

fn command_name(args: &[String]) -> &str {
    match args.first().map(String::as_str) {
        Some(c @ ("login" | "logout" | "status" | "keys")) => c,
        _ => "account",
    }
}

fn finish(json: bool, command: &str, result: Result<Outcome, CliError>) -> i32 {
    let out = result.unwrap_or_else(|e| Outcome {
        json: json!({ "ok": false, "command": command, "error": { "code": e.code, "exit": e.exit, "message": e.message } }),
        text: String::new(),
        exit: e.exit,
    });
    match (json, out.exit, out.json["error"]["message"].as_str()) {
        (true, ..) => println!("{}", out.json),
        (false, _, Some(message)) => eprintln!("gnobbi: {message}"),
        (false, _, None) => println!("{}", out.text),
    }
    out.exit
}

/// Entry point of `gnobbi account …` (arguments after `account`). Returns the exit code.
pub fn main(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let command = command_name(args);
    let password = take_env("GNOBBI_PASSWORD");
    let env = |key: &str| match key {
        "GNOBBI_PASSWORD" => password.as_ref().map(|_| String::new()),
        _ => std::env::var(key).ok(),
    };
    let parsed = match parse(args, &env) {
        Ok(p) => p,
        Err(message) => return finish(json, command, Err(usage(message))),
    };
    let work = async {
        let ca = parsed.ca_file.as_deref().map(read_ca).transpose()?;
        let dirs = app_dirs(Platform::CURRENT, |k| std::env::var(k).ok()).ok_or_else(|| storage("no user directories (HOME unset?)"))?;
        match parsed.cmd {
            Cmd::Login { user, homeserver, source } => {
                let secret = read_password(&source, password)?;
                login(&dirs, user, homeserver, secret, ca).await
            }
            Cmd::Logout { account, local } => logout(&dirs, &account, local, ca).await,
            Cmd::Status { account } => status(&dirs, account.as_deref(), ca).await,
            Cmd::Keys => Err(usage("keys: not implemented yet")),
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
    let result = match runtime {
        Ok(rt) => rt.block_on(work),
        Err(_) => Err(fail(1, "internal", "cannot start the async runtime")),
    };
    finish(parsed.json, command, result)
}

#[cfg(test)]
mod tests;
