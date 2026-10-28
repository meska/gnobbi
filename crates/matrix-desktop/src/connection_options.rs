use matrix_domain::AccountId;
use std::path::PathBuf;

/// Graphical sign-in for an explicit isolated local profile. No secret in argv.
pub struct SignInOptions {
    pub root: PathBuf,
    pub ca_file: Option<PathBuf>,
}
impl SignInOptions {
    pub fn parse(args: &[String]) -> Result<Option<Self>, &'static str> {
        if args.first().is_none_or(|arg| arg != "--signin") {
            return Ok(None);
        }
        if !matches!(args.len(), 2 | 4) || (args.len() == 4 && args[2] != "--ca-file") {
            return Err("Use --signin ABSOLUTE_PROFILE_ROOT [--ca-file ABSOLUTE_PEM]");
        }
        let root = PathBuf::from(&args[1]);
        let ca_file = args.get(3).map(PathBuf::from);
        if !root.is_absolute() || ca_file.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err("Sign-in paths must be absolute");
        }
        Ok(Some(Self { root, ca_file }))
    }
}

/// Explicit restore only. No automatic personal path, login or secret in argv.
pub struct RestoreOptions {
    pub root: PathBuf,
    pub account_id: AccountId,
    pub ca_file: Option<PathBuf>,
}
impl RestoreOptions {
    pub fn parse(args: &[String]) -> Result<Option<Self>, &'static str> {
        if args.is_empty() {
            return Ok(None);
        }
        if !matches!(args.len(), 3 | 5) || args[0] != "--restore" {
            return Err("Use --restore ABSOLUTE_STORE_ROOT ACCOUNT_ID [--ca-file ABSOLUTE_PEM]");
        }
        let root = PathBuf::from(&args[1]);
        let account_id = AccountId::new(args[2].parse().map_err(|_| "Invalid local account ID")?);
        let ca_file = if args.len() == 5 {
            if args[3] != "--ca-file" {
                return Err("Unsupported restore option");
            }
            Some(PathBuf::from(&args[4]))
        } else {
            None
        };
        if !root.is_absolute() || ca_file.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err("Restore paths must be absolute");
        }
        Ok(Some(Self {
            root,
            account_id,
            ca_file,
        }))
    }
}

/// Launched from a `.app` bundle with no arguments (Finder, TestFlight): open the connected
/// sign-in on the app's own profile directory instead of the offline demo.
/// `home` is the app container HOME when sandboxed. Explicit arguments always win.
pub fn bundle_launch_args(exe: &str, home: Option<&str>, args: &[String]) -> Option<Vec<String>> {
    if !args.is_empty() || !exe.contains(".app/Contents/MacOS/") {
        return None;
    }
    let home = home.filter(|home| home.starts_with('/'))?;
    let profile = format!(
        "{}/Library/Application Support/Dime/profile",
        home.trim_end_matches('/')
    );
    Some(vec!["--signin".to_owned(), profile])
}

/// Linux and Windows binaries started with no arguments: open the connected sign-in like the macOS bundle.
pub fn linux_launch_args(args: &[String]) -> Vec<String> {
    if args.is_empty() {
        vec!["--signin".to_owned()]
    } else {
        args.to_vec()
    }
}

/// `--signin` without a profile path (alone or followed by `--ca-file`) opens the profile under the app data directory.
pub fn default_signin_args(args: &[String], dirs: &crate::AppDirs) -> Vec<String> {
    match args {
        [flag, rest @ ..]
            if flag == "--signin" && rest.first().is_none_or(|a| a == "--ca-file") =>
        {
            let profile = dirs.data.join("profile").to_string_lossy().into_owned();
            [vec![flag.clone(), profile], rest.to_vec()].concat()
        }
        _ => args.to_vec(),
    }
}

/// Create `path` (and missing parents) readable by the owner only; the profile store refuses looser modes.
pub fn create_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    // Windows has no mode bits: the user-profile ACL already keeps the folder private.
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}
