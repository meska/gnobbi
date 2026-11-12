//! Windows: per-user registration of the `matrix:` scheme (HKCU, silent, no UserChoice) and one
//! running Gnobbi per session. A second launch hands its link (or just "come to front") to the live
//! one over a per-user named pipe and exits before the UI starts.
//! Wire format (same as the Linux socket): the link as text + `\n` (empty = only raise the window),
//! at most [`MAX_LINK`] bytes. The pure parts compile everywhere so they are testable off Windows.
use std::{io::Read, path::Path};

pub const MAX_LINK: usize = 2048;

/// One registry string value under `HKCU`.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub subkey: &'static str,
    pub name: &'static str,
    pub value: String,
}

/// `"<exe>" "%1"`: both quoted, so a path or a link with spaces stays one argument.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

pub fn desired(exe: &Path) -> Vec<Entry> {
    let (class, cmd) = (r"Software\Classes\matrix", r"Software\Classes\matrix\shell\open\command");
    let entry = |subkey, name, value: &str| Entry { subkey, name, value: value.to_owned() };
    vec![
        entry(class, "", "URL:Matrix"),
        entry(class, "URL Protocol", ""),
        Entry { subkey: cmd, name: "", value: command_line(exe) },
    ]
}

/// The entries `read` does not already report with the desired value: empty when up to date, so
/// running it at every start writes nothing unless the exe moved.
pub fn stale(exe: &Path, read: impl Fn(&Entry) -> Option<String>) -> Vec<Entry> {
    desired(exe).into_iter().filter(|e| read(e).as_deref() != Some(e.value.as_str())).collect()
}

/// The wire bytes for `link`, `None` when it is too long or multi-line.
pub fn encode(link: &str) -> Option<Vec<u8>> {
    (link.len() <= MAX_LINK && !link.contains('\n')).then(|| format!("{link}\n").into_bytes())
}

/// Reads one message: `Some(link)` (possibly empty), `None` when too long or not UTF-8.
pub fn decode(r: impl Read) -> Option<String> {
    let mut buf = Vec::new();
    // Reading a little over the limit tells "too long" without reading an unbounded stream.
    r.take(MAX_LINK as u64 + 2).read_to_end(&mut buf).ok()?;
    let line = String::from_utf8(buf).ok()?;
    let link = line.strip_suffix('\n')?;
    (link.len() <= MAX_LINK && !link.contains('\n')).then(|| link.to_owned())
}

#[cfg(windows)]
pub use sys::{Claim, claim, register_for_current_user, take_incoming};

#[cfg(windows)]
mod sys {
    use super::*;
    use std::{
        fs::{File, OpenOptions},
        io::{self, Write},
        os::windows::io::{AsRawHandle, FromRawHandle},
        sync::Mutex,
        time::Duration,
    };
    use windows::{
        Win32::{
            Foundation::{
                ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED, ERROR_SUCCESS, GetLastError, HANDLE,
                INVALID_HANDLE_VALUE,
            },
            Security::{
                Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
                PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
            },
            Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_INBOUND},
            System::{
                Pipes::{
                    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
                    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
                },
                Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW},
                Threading::CreateMutexW,
            },
            UI::WindowsAndMessaging::AllowSetForegroundWindow,
        },
        core::{HSTRING, w},
    };

    fn read(e: &Entry) -> Option<String> {
        let (sub, name) = (HSTRING::from(e.subkey), HSTRING::from(e.name));
        let mut buf = [0u16; 1024];
        let mut bytes = std::mem::size_of_val(&buf) as u32;
        // SAFETY: `buf` and `bytes` outlive the call and `bytes` is its size in bytes.
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER, &sub, &name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut bytes),
            )
        };
        // `bytes` counts the terminating NUL.
        (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(bytes as usize / 2).saturating_sub(1)]))
    }

    fn write(e: &Entry) -> Result<(), u32> {
        let (sub, name) = (HSTRING::from(e.subkey), HSTRING::from(e.name));
        let data: Vec<u16> = e.value.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `data` outlives the call; the size is in bytes and includes the NUL.
        let r = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER, &sub, &name, REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32,
            )
        };
        if r == ERROR_SUCCESS { Ok(()) } else { Err(r.0) }
    }

    /// Makes `matrix:` open this exe, for the current user only. Silent; a write failure logs the
    /// error code and the app starts anyway.
    pub fn register_for_current_user() {
        let Ok(exe) = std::env::current_exe() else { return };
        for entry in stale(&exe, read) {
            if let Err(code) = write(&entry) {
                eprintln!("Gnobbi: matrix: registration failed (error {code})");
                return;
            }
        }
    }

    fn pipe_name() -> String {
        let user = std::env::var("USERNAME").unwrap_or_default();
        let user: String = user.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        format!(r"\\.\pipe\gnobbi-{user}")
    }

    pub enum Claim {
        /// We are the first instance and the pipe server runs.
        First,
        /// A live instance took the message; this process must exit.
        Delivered,
    }

    /// Becomes the first instance (mutex + pipe server) or delivers `link` (empty = raise only) to it.
    /// The mutex dies with its process, so a crashed first instance never blocks the next launch.
    pub fn claim(link: &str) -> io::Result<Claim> {
        let msg = encode(link).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "link too long or multi-line"))?;
        // SAFETY: plain FFI; the handle is kept open for the whole life of the process on purpose.
        let mutex = unsafe { CreateMutexW(None, false, w!(r"Local\gnobbi-single-instance")) }?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            deliver(&msg)?;
            return Ok(Claim::Delivered);
        }
        let _ = mutex;
        serve()?;
        Ok(Claim::First)
    }

    fn deliver(msg: &[u8]) -> io::Result<()> {
        // Let the live instance take the foreground (this process was started by the shell, it may).
        let _ = unsafe { AllowSetForegroundWindow(u32::MAX) };
        // 2 = pipe not created yet (first instance still starting), 231 = every instance busy.
        for _ in 0..30 {
            // SECURITY_IDENTIFICATION (0x10000): a squatter owning the pipe name can read the link but not impersonate us.
            match std::os::windows::fs::OpenOptionsExt::security_qos_flags(OpenOptions::new().write(true), 0x0001_0000).open(pipe_name()) {
                Ok(mut pipe) => return pipe.write_all(msg),
                Err(e) if matches!(e.raw_os_error(), Some(2 | 231)) => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => return Err(e),
            }
        }
        Err(io::ErrorKind::TimedOut.into())
    }

    /// One server instance of the pipe; the DACL grants the owner only (no other user, no network).
    fn create(sd: usize, first: bool) -> io::Result<File> {
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd as *mut _,
            bInheritHandle: false.into(),
        };
        let open = PIPE_ACCESS_INBOUND | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { FILE_FLAGS_AND_ATTRIBUTES(0) };
        // SAFETY: `attrs` outlives the call; the handle is owned by the returned File.
        let h = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(pipe_name()),
                open,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                0,
                4096,
                0,
                Some(&attrs),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_handle(h.0) })
    }

    static INCOMING: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn serve() -> io::Result<()> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // D:P = no inherited ACEs; OW (owner rights) = full access for the creating user, nobody else.
        // SAFETY: `sd` is a valid out-pointer; the descriptor is intentionally never freed.
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w!("D:P(A;;GA;;;OW)"), SDDL_REVISION_1, &mut sd, None) }?;
        let sd = sd.0 as usize;
        let mut current = create(sd, true)?;
        std::thread::spawn(move || {
            loop {
                // SAFETY: the handle is owned by `current`, alive for the call.
                let connected = unsafe { ConnectNamedPipe(HANDLE(current.as_raw_handle()), None) };
                // A client that connected before this call reports ERROR_PIPE_CONNECTED: also fine.
                let ok = connected.is_ok() || connected.is_err_and(|e| e.code() == ERROR_PIPE_CONNECTED.to_hresult());
                let Ok(next) = create(sd, false) else { return };
                let conn = std::mem::replace(&mut current, next);
                if ok {
                    // shortcut: a same-user client that connects and never writes parks one thread; add a timeout if it matters.
                    std::thread::spawn(move || {
                        if let Some(link) = decode(conn)
                            && let Ok(mut queue) = INCOMING.lock()
                        {
                            queue.push(link);
                        }
                    });
                }
            }
        });
        Ok(())
    }

    /// Messages received since the last call; the UI tick drains it.
    pub fn take_incoming() -> Vec<String> {
        INCOMING.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn command_line_quotes_exe_and_link() {
        assert_eq!(command_line(Path::new(r"C:\Program Files\Gnobbi\gnobbi.exe")), r#""C:\Program Files\Gnobbi\gnobbi.exe" "%1""#);
    }

    #[test]
    fn desired_entries_are_the_three_hkcu_values() {
        let d = desired(Path::new(r"C:\g\gnobbi.exe"));
        let got: Vec<_> = d.iter().map(|e| (e.subkey, e.name, e.value.as_str())).collect();
        assert_eq!(
            got,
            [
                (r"Software\Classes\matrix", "", "URL:Matrix"),
                (r"Software\Classes\matrix", "URL Protocol", ""),
                (r"Software\Classes\matrix\shell\open\command", "", r#""C:\g\gnobbi.exe" "%1""#),
            ]
        );
    }

    #[test]
    fn registration_is_idempotent_and_follows_the_exe() {
        let (a, b) = (Path::new(r"C:\a\gnobbi.exe"), Path::new(r"C:\b\gnobbi.exe"));
        // Nothing registered: everything is written.
        assert_eq!(stale(a, |_| None).len(), 3);
        // A store holding what `a` wrote: nothing to do for `a`, only the command for `b`.
        let store = desired(a);
        let read = |e: &Entry| store.iter().find(|s| (s.subkey, s.name) == (e.subkey, e.name)).map(|s| s.value.clone());
        assert!(stale(a, read).is_empty());
        let moved = stale(b, read);
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].value, command_line(b));
    }

    #[test]
    fn pipe_message_roundtrip_and_limits() {
        assert_eq!(decode(Cursor::new(encode("matrix:u/ada:x.test").unwrap())).as_deref(), Some("matrix:u/ada:x.test"));
        // Empty = only raise.
        assert_eq!(decode(Cursor::new(encode("").unwrap())).as_deref(), Some(""));
        assert!(encode(&"a".repeat(MAX_LINK + 1)).is_none());
        assert!(encode("a\nb").is_none());
        assert_eq!(decode(Cursor::new(encode(&"a".repeat(MAX_LINK)).unwrap())).map(|s| s.len()), Some(MAX_LINK));
        // A hostile client skips our check: the receiver still drops it, and does not need the newline.
        assert_eq!(decode(Cursor::new([vec![b'a'; 5000], b"\n".to_vec()].concat())), None);
        assert_eq!(decode(Cursor::new(b"matrix:u/a:b".to_vec())), None);
        assert_eq!(decode(Cursor::new(vec![0xff, b'\n'])), None);
    }
}
