//! One running Gnobbi per user (Linux): a second launch hands its `matrix:` link (or just "come to
//! front") to the live one over a local Unix socket and exits before the UI starts. Std only.
//! Wire format: one line, the link as text (empty = only raise the window), at most [`MAX_LINK`] bytes.
use std::{
    io::{self, Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const MAX_LINK: usize = 2048;

/// `$XDG_RUNTIME_DIR`, else `$XDG_STATE_HOME`, else `/tmp` with the uid in the name.
pub fn socket_path(runtime: Option<&Path>, state: Option<&Path>, uid: u32) -> PathBuf {
    match runtime.or(state).filter(|p| p.is_absolute()) {
        Some(dir) => dir.join("gnobbi.sock"),
        None => PathBuf::from(format!("/tmp/gnobbi-{uid}.sock")),
    }
}

/// The path from the environment (uid taken from the owner of `$HOME`: no unsafe, no libc).
pub fn default_socket_path() -> PathBuf {
    let var = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let uid = var("HOME")
        .and_then(|h| std::fs::metadata(h).ok())
        .map_or(0, |m| m.uid());
    socket_path(var("XDG_RUNTIME_DIR").as_deref(), var("XDG_STATE_HOME").as_deref(), uid)
}

pub enum Claim {
    /// No live instance: we are the first and own the listener.
    First(UnixListener),
    /// A live instance took the message; this process must exit.
    Delivered,
}

/// Connects to a live instance and delivers `link` (empty = raise only), else becomes the first.
/// A socket nobody answers on (crash of the first instance) is removed and replaced.
pub fn claim(path: &Path, link: &str) -> io::Result<Claim> {
    if link.len() > MAX_LINK || link.contains('\n') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "link too long or multi-line"));
    }
    for _ in 0..2 {
        match UnixStream::connect(path) {
            Ok(mut s) => {
                s.write_all(format!("{link}\n").as_bytes())?;
                return Ok(Claim::Delivered);
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => {
                let _ = std::fs::remove_file(path);
                match UnixListener::bind(path) {
                    Ok(l) => {
                        // shortcut: bind then chmod leaves a brief window; /tmp fallback only, upgrade to a 0700 dir if it matters.
                        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                        return Ok(Claim::First(l));
                    }
                    // Another first launch won the race: connect to it on the next round.
                    Err(e) if e.kind() == io::ErrorKind::AddrInUse => {}
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("could not claim the socket"))
}

/// Reads one message: `Some(link)` (possibly empty), `None` when too long or not UTF-8.
fn read_message(mut s: UnixStream) -> Option<String> {
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
        d
    }

    fn receive(l: &UnixListener) -> Option<String> {
        read_message(l.accept().unwrap().0)
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
    fn path_prefers_runtime_then_state_then_tmp() {
        let (r, s) = (Path::new("/run/user/1"), Path::new("/s"));
        assert_eq!(socket_path(Some(r), Some(s), 7), Path::new("/run/user/1/gnobbi.sock"));
        assert_eq!(socket_path(None, Some(s), 7), Path::new("/s/gnobbi.sock"));
        assert_eq!(socket_path(None, None, 7), Path::new("/tmp/gnobbi-7.sock"));
    }
}
