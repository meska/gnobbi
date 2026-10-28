//! Bounded local OS notification adapter, separate from the Matrix runtime.
use matrix_domain::{AccountId, RoomScope, notifications::Notification};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationError {
    Unavailable,
    Timeout,
    Worker,
}

/// The last clicked alert: its catalog account (None: the shown one) and room.
type Click = Arc<Mutex<Option<(Option<AccountId>, RoomScope)>>>;

#[derive(Clone)]
pub struct NotificationHandle {
    click: Click,
    sender: mpsc::SyncSender<(u64, Notification)>,
    revision: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
}
impl NotificationHandle {
    /// Nonblocking, best effort: one pending alert maximum.
    pub fn submit(&self, notification: Notification) -> bool {
        self.sender
            .try_send((self.revision.load(Ordering::Acquire), notification))
            .is_ok()
    }
    /// Invalidates queued alerts, not those already handed to the OS service.
    pub fn cancel_pending(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }
    /// The alert the user clicked since the last call, if any (only the latest is kept).
    pub fn take_click(&self) -> Option<(Option<AccountId>, RoomScope)> {
        self.click.lock().ok()?.take()
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
}

pub struct NotificationWorker {
    handle: NotificationHandle,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl NotificationWorker {
    pub fn spawn() -> std::io::Result<Self> {
        let click = Click::default();
        let slot = click.clone();
        Self::spawn_clicks(click, move |n| deliver_os(n, &slot))
    }
    #[cfg(test)]
    fn spawn_with<F>(deliver: F) -> std::io::Result<Self>
    where
        F: FnMut(Notification) -> Result<(), NotificationError> + Send + 'static,
    {
        Self::spawn_clicks(Click::default(), deliver)
    }
    fn spawn_clicks<F>(click: Click, mut deliver: F) -> std::io::Result<Self>
    where
        F: FnMut(Notification) -> Result<(), NotificationError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let revision = Arc::new(AtomicU64::new(0));
        let current = revision.clone();
        let failed = Arc::new(AtomicBool::new(false));
        let failure = failed.clone();
        let thread = std::thread::Builder::new()
            .name("matrix-notifications".into())
            .spawn(move || {
                while !cancelled.load(Ordering::Acquire) {
                    match receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok((version, notification)) => {
                            if !cancelled.load(Ordering::Acquire)
                                && version == current.load(Ordering::Acquire)
                            {
                                failure.store(deliver(notification).is_err(), Ordering::Release);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })?;
        Ok(Self {
            handle: NotificationHandle {
                click,
                sender,
                revision,
                failed,
            },
            stop,
            thread: Some(thread),
        })
    }
    pub fn handle(&self) -> NotificationHandle {
        self.handle.clone()
    }
    pub fn shutdown(mut self) -> Result<(), NotificationError> {
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .unwrap()
            .join()
            .map_err(|_| NotificationError::Worker)
    }
}
impl Drop for NotificationWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Blocking platform boundary: only the notification worker may call this. The alert carries an
/// action; a short-lived thread per alert (at most `WAITERS` at once) reports a click.
#[cfg(target_os = "linux")]
fn deliver_os(notification: Notification, click: &Click) -> Result<(), NotificationError> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    static WAITERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    const WAITERS_MAX: usize = 4;
    let mut child = Command::new("/usr/bin/notify-send")
        .args([
            "--app-name=Gnobbi",
            "--urgency=low",
            "--expire-time=5000",
            "--hint=boolean:suppress-sound:true",
            "--hint=boolean:transient:true",
            "--category=im.received",
            "--action=open=Apri",
            "--wait",
            "--",
            notification.title(),
            &notification.body(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| NotificationError::Unavailable)?;
    // A failure (no service, unknown option) shows within a moment; a live alert keeps running.
    let early = std::time::Instant::now() + Duration::from_millis(300);
    while std::time::Instant::now() < early {
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => return Err(NotificationError::Unavailable),
            Ok(Some(_)) => break,
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let (account, scope) = {
        let (account, scope) = notification.target();
        (account, scope.clone())
    };
    if WAITERS.fetch_add(1, Ordering::AcqRel) >= WAITERS_MAX {
        WAITERS.fetch_sub(1, Ordering::AcqRel);
        let _ = child.kill();
        let _ = child.wait();
        return Ok(());
    }
    let click = click.clone();
    std::thread::spawn(move || {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        let done = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if std::time::Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
            }
        };
        let mut out = String::new();
        if done && let Some(pipe) = child.stdout.as_mut() {
            let _ = pipe.take(64).read_to_string(&mut out);
        }
        if out.trim() == "open"
            && let Ok(mut slot) = click.lock()
        {
            *slot = Some((account, scope));
        }
        WAITERS.fetch_sub(1, Ordering::AcqRel);
    });
    Ok(())
}
/// Fixed AppleScript; title/body travel only as argv, so message text can never become script.
#[cfg(any(target_os = "macos", test))]
fn macos_osascript_args(notification: &Notification) -> Vec<String> {
    vec![
        "-e".into(),
        "on run argv".into(),
        "-e".into(),
        "display notification (item 2 of argv) with title (item 1 of argv)".into(),
        "-e".into(),
        "end run".into(),
        "--".into(),
        notification.title().into(),
        notification.body(),
    ]
}

/// Blocking platform boundary: only the notification worker may call this.
#[cfg(target_os = "macos")]
// shortcut: osascript alerts report no click, so on macOS they only name the account.
fn deliver_os(notification: Notification, _: &Click) -> Result<(), NotificationError> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("/usr/bin/osascript")
        .args(macos_osascript_args(&notification))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| NotificationError::Unavailable)?;
    let end = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(NotificationError::Unavailable),
            Ok(None) if std::time::Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(NotificationError::Timeout);
            }
        }
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn deliver_os(_: Notification, _: &Click) -> Result<(), NotificationError> {
    Err(NotificationError::Unavailable)
}

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
