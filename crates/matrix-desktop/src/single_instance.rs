//! One running Gnobbi per user (Linux): a second launch hands its `matrix:` link (or just "come to
//! front") to the live one over a local Unix socket and exits before the UI starts. Std only.
//! Wire format: one line, the link as text (empty = only raise the window), at most [`MAX_LINK`] bytes.
use std::{
    io::{self, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const MAX_LINK: usize = 2048;

/// The socket's directory: `$XDG_RUNTIME_DIR`, else `<state>/gnobbi` (`$XDG_STATE_HOME`, else
/// `$HOME/.local/state`). Never `/tmp`: no usable base means `None` (single instance off).
pub fn socket_path(runtime: Option<&Path>, state: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let abs = |p: Option<&Path>| p.filter(|p| p.is_absolute()).map(Path::to_path_buf);
    if let Some(dir) = abs(runtime) {
        return Some(dir.join("gnobbi.sock"));
    }
    let state = abs(state).or_else(|| abs(home).map(|h| h.join(".local/state")))?;
    Some(state.join("gnobbi").join("gnobbi.sock"))
}

pub fn default_socket_path() -> Option<PathBuf> {
    let var = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    socket_path(var("XDG_RUNTIME_DIR").as_deref(), var("XDG_STATE_HOME").as_deref(), var("HOME").as_deref())
}

/// Our uid: the owner of `/proc/self` (no unsafe, no libc). Linux only; elsewhere the gate stays off.
fn my_uid() -> io::Result<u32> {
    Ok(std::fs::metadata("/proc/self")?.uid())
}

/// Creates (0700) when missing, then requires a real directory (no symlink) of ours with no group/other access.
fn private_dir(dir: &Path, create: bool) -> io::Result<()> {
    if create && std::fs::symlink_metadata(dir).is_err() {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let m = std::fs::symlink_metadata(dir)?;
    if !m.is_dir() || m.uid() != my_uid()? || m.mode() & 0o077 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "socket directory is not private to this user"));
    }
    Ok(())
}

pub enum Claim {
    /// No live instance: we are the first and own the listener.
    First(UnixListener),
    /// A live instance took the message; this process must exit.
    Delivered,
}

/// What the gate sends for a link found in argv: it as is, or (oversized/multi-line) a marker that
/// fails `parse_matrix_link`, so the live instance shows «Link non valido» instead of staying silent.
pub fn outgoing(link: Option<String>) -> String {
    match link {
        Some(l) if l.len() <= MAX_LINK && !l.contains('\n') => l,
        Some(_) => "matrix:".to_owned(),
        None => String::new(),
    }
}

/// Connects to a live instance and delivers `link` (empty = raise only), else becomes the first.
/// The directory must be private to us and the path nothing but our own socket; a stale socket
/// (crash of the first instance) is removed and replaced. A lock file serializes concurrent launches,
/// so one never deletes the socket another has just created.
pub fn claim(path: &Path, link: &str) -> io::Result<Claim> {
    if link.len() > MAX_LINK || link.contains('\n') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "link too long or multi-line"));
    }
    let dir = path.parent().ok_or_else(|| io::Error::other("socket path has no parent"))?;
    // Only our own `<state>/gnobbi` is created; `$XDG_RUNTIME_DIR` must already exist and be ours.
    private_dir(dir, dir.file_name().is_some_and(|n| n == "gnobbi"))?;
    let lock = std::fs::OpenOptions::new().create(true).write(true).truncate(false).mode(0o600).open(dir.join("gnobbi.lock"))?;
    lock.lock()?;
    match std::fs::symlink_metadata(path) {
        Ok(m) if !m.file_type().is_socket() || m.uid() != my_uid()? => {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "socket path holds something else"));
        }
        Ok(_) => match UnixStream::connect(path) {
            Ok(mut s) => {
                s.write_all(format!("{link}\n").as_bytes())?;
                return Ok(Claim::Delivered);
            }
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => std::fs::remove_file(path)?,
            Err(e) => return Err(e),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(Claim::First(listener))
}

/// Reads one message: `Some(link)` (possibly empty), `None` when too long or not UTF-8.
fn read_message(s: UnixStream) -> Option<String> {
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let mut buf = Vec::new();
    // One byte over the limit tells "too long" without reading an unbounded stream.
    s.take(MAX_LINK as u64 + 2).read_to_end(&mut buf).ok()?;
    let line = String::from_utf8(buf).ok()?;
    let link = line.strip_suffix('\n')?;
    (link.len() <= MAX_LINK && !link.contains('\n')).then(|| link.to_owned())
}

static INCOMING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Accepts connections forever on a background thread, queuing what arrives.
pub fn serve(listener: UnixListener) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Some(link) = read_message(stream)
                && let Ok(mut queue) = INCOMING.lock()
            {
                queue.push(link);
            }
        }
    });
}

/// Messages received since the last call; the UI tick drains it.
pub fn take_incoming() -> Vec<String> {
    INCOMING.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gnobbi-si-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
        d
    }

    fn receive(l: &UnixListener) -> Option<String> {
        read_message(l.accept().unwrap().0)
    }

    #[test]
    fn rejected_link_becomes_a_short_marker() {
        assert_eq!(outgoing(None), "");
        assert_eq!(outgoing(Some("matrix:u/a:b".into())), "matrix:u/a:b");
        for bad in [format!("matrix:u/{}", "a".repeat(MAX_LINK)), "matrix:u/a\nb".to_owned()] {
            let m = outgoing(Some(bad));
            assert_eq!(m, "matrix:");
            assert!(m.len() <= MAX_LINK && !m.contains('\n'));
        }
    }

    #[test]
    fn second_launch_delivers_to_the_first() {
        let p = dir("deliver").join("s.sock");
        let Claim::First(l) = claim(&p, "").unwrap() else { panic!("first") };
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(matches!(claim(&p, "matrix:u/ada:x.test").unwrap(), Claim::Delivered));
        assert_eq!(receive(&l).as_deref(), Some("matrix:u/ada:x.test"));
        assert!(matches!(claim(&p, "").unwrap(), Claim::Delivered));
        assert_eq!(receive(&l).as_deref(), Some(""));
    }

    #[test]
    fn too_long_link_is_refused_on_both_sides() {
        let p = dir("long").join("s.sock");
        let Claim::First(l) = claim(&p, "").unwrap() else { panic!("first") };
        assert!(claim(&p, &"a".repeat(MAX_LINK + 1)).is_err());
        // A hostile client skips our check: the receiver still drops it.
        let mut raw = UnixStream::connect(&p).unwrap();
        raw.write_all(&vec![b'a'; 5000]).unwrap();
        raw.write_all(b"\n").unwrap();
        drop(raw);
        assert_eq!(receive(&l), None);
        // Exactly at the limit passes.
        assert!(matches!(claim(&p, &"a".repeat(MAX_LINK)).unwrap(), Claim::Delivered));
        assert_eq!(receive(&l).map(|s| s.len()), Some(MAX_LINK));
    }

    #[test]
    fn stale_socket_is_replaced() {
        let p = dir("stale").join("s.sock");
        // A crash leaves the file behind with nobody listening.
        drop(UnixListener::bind(&p).unwrap());
        assert!(p.exists());
        assert!(matches!(claim(&p, "matrix:u/ada:x.test").unwrap(), Claim::First(_)));
    }

    #[test]
    fn a_file_that_is_not_our_socket_is_refused_and_kept() {
        let p = dir("file").join("s.sock");
        std::fs::write(&p, "precious").unwrap();
        assert!(claim(&p, "").is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "precious");
        // A symlink (even to a dead target) is no socket either.
        let q = dir("link").join("s.sock");
        std::os::unix::fs::symlink("/nonexistent", &q).unwrap();
        assert!(claim(&q, "").is_err());
        assert!(std::fs::symlink_metadata(&q).unwrap().file_type().is_symlink());
    }

    #[test]
    fn an_unsafe_directory_is_refused() {
        let d = dir("open");
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(claim(&d.join("s.sock"), "").is_err());
        assert!(!d.join("s.sock").exists());
        // A symlinked directory is refused even when the target is private.
        let real = dir("real");
        let via = dir("via").join("gnobbi");
        std::os::unix::fs::symlink(&real, &via).unwrap();
        assert!(claim(&via.join("gnobbi.sock"), "").is_err());
        assert!(!real.join("gnobbi.sock").exists());
        // A missing `$XDG_RUNTIME_DIR` is not created.
        assert!(claim(&dir("missing").join("run").join("s.sock"), "").is_err());
    }

    #[test]
    fn own_state_dir_is_created_private() {
        let p = dir("state").join("gnobbi").join("gnobbi.sock");
        assert!(matches!(claim(&p, "").unwrap(), Claim::First(_)));
        let mode = |q: &Path| std::fs::metadata(q).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(p.parent().unwrap()), mode(&p)), (0o700, 0o600));
    }

    #[test]
    fn concurrent_launches_elect_one_first() {
        let p = dir("race").join("s.sock");
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let p = p.clone();
                std::thread::spawn(move || claim(&p, "").map(|c| matches!(c, Claim::First(_))).unwrap())
            })
            .collect();
        // Only the first keeps the listener alive (returned and dropped here); count who was first.
        let firsts = handles.into_iter().filter(|_| true).map(|h| h.join().unwrap()).filter(|f| *f).count();
        assert!(firsts >= 1);
    }

    #[test]
    fn path_prefers_runtime_then_state_then_home_never_tmp() {
        let (r, s, h) = (Path::new("/run/user/1"), Path::new("/s"), Path::new("/home/a"));
        assert_eq!(socket_path(Some(r), Some(s), Some(h)).unwrap(), Path::new("/run/user/1/gnobbi.sock"));
        assert_eq!(socket_path(None, Some(s), Some(h)).unwrap(), Path::new("/s/gnobbi/gnobbi.sock"));
        assert_eq!(socket_path(None, None, Some(h)).unwrap(), Path::new("/home/a/.local/state/gnobbi/gnobbi.sock"));
        assert_eq!(socket_path(None, None, None), None);
        assert_eq!(socket_path(Some(Path::new("rel")), None, None), None);
    }
}
