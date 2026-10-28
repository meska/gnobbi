//! Owner-only file helpers shared by the profile stores. Unix: mode bits. Windows: no mode
//! bits exist; stores live under %APPDATA%/%LOCALAPPDATA%, whose inherited ACL already limits
//! them to the user, SYSTEM and Administrators.
// shortcut: no explicit ACL on Windows and no hard-link-count check (std has it unstable); add both if stores ever live outside the user profile.
use std::{fs, io, path::Path};

/// New files are created owner-only (0600 on Unix).
pub(crate) fn private_file(opts: &mut fs::OpenOptions) -> &mut fs::OpenOptions {
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(opts, 0o600);
    opts
}

/// Create one directory owner-only (0700 on Unix); the parent must exist.
pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// Force an existing directory back to owner-only.
pub(crate) fn tighten_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    return fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    #[cfg(not(unix))]
    fs::metadata(path).map(|_| ())
}

/// Exactly the owner-only mode (0700 dir / 0600 file); always true where there are no mode bits.
pub(crate) fn mode_is(meta: &fs::Metadata, expected: u32) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o777 == expected;
    #[cfg(not(unix))]
    {
        let _ = (meta, expected);
        true
    }
}

/// No second hard link to the file.
pub(crate) fn single_link(meta: &fs::Metadata) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::MetadataExt::nlink(meta) == 1;
    #[cfg(not(unix))]
    {
        let _ = meta;
        true
    }
}

/// `path` is already its own canonical form (no symlink or `..` hop on the way).
pub(crate) fn is_canonical(path: &Path) -> bool {
    match fs::canonicalize(path) {
        // Windows: canonicalize returns a `\\?\` verbatim path and the real letter case, and NTFS is case-insensitive.
        #[cfg(windows)]
        Ok(real) => strip_verbatim(&real)
            .to_string_lossy()
            .eq_ignore_ascii_case(&strip_verbatim(path).to_string_lossy()),
        #[cfg(not(windows))]
        Ok(real) => real == path,
        Err(_) => false,
    }
}

#[cfg(windows)]
fn strip_verbatim(path: &Path) -> &Path {
    path.to_str()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .map_or(path, Path::new)
}

/// Flush a directory entry to disk. A no-op on Windows, which cannot open directories as files.
pub(crate) fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    return fs::File::open(path).and_then(|dir| dir.sync_all());
    #[cfg(not(unix))]
    fs::metadata(path).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn private_files_and_dirs_round_trip_and_reject_loose_modes() {
        let root = std::env::temp_dir().join(format!("gnobbi-fsx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        create_private_dir(&root).unwrap();
        assert!(is_canonical(&root));
        #[cfg(windows)]
        assert!(is_canonical(Path::new(
            &root.to_string_lossy().to_uppercase()
        )));
        assert!(mode_is(&fs::symlink_metadata(&root).unwrap(), 0o700));
        let file = root.join("f");
        private_file(fs::OpenOptions::new().write(true).create_new(true))
            .open(&file)
            .unwrap()
            .write_all(b"x")
            .unwrap();
        let meta = fs::symlink_metadata(&file).unwrap();
        assert!(meta.is_file() && mode_is(&meta, 0o600) && single_link(&meta));
        assert!(create_private_dir(&root).is_err());
        sync_dir(&root).unwrap();
        tighten_dir(&root).unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            assert!(!mode_is(&fs::symlink_metadata(&root).unwrap(), 0o700));
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
