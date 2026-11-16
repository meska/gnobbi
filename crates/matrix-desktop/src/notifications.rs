//! Bounded local OS notification adapter, separate from the Matrix runtime.
use matrix_domain::{AccountId, RoomScope, notifications::Notification};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::Duration;

/// Controls of the «Notifiche locali» panel, in Tab order; the keyboard focus lives in every build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotificationControl {
    #[default]
    Enabled,
    Mute,
    Preview,
    MessagePreview,
    Close,
}
impl NotificationControl {
    /// Next control in Tab order, wrapping; «Mute» is skipped while no room is selected.
    pub fn next(self, room: bool, backwards: bool) -> Self {
        use NotificationControl::*;
        let all = [Enabled, Mute, Preview, MessagePreview, Close];
        let controls: Vec<_> = all.into_iter().filter(|c| room || *c != Mute).collect();
        let index = controls.iter().position(|c| *c == self).unwrap_or(0);
        controls[(index + if backwards { controls.len() - 1 } else { 1 }) % controls.len()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationError {
    Unavailable,
    Timeout,
    Worker,
}

/// True when the alert is for the room the user is looking at in a focused window; other rooms and accounts still alert.
pub fn reading(window_active: bool, selected: Option<&RoomScope>, alert: &RoomScope) -> bool {
    window_active && selected == Some(alert)
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

/// Linux: the freedesktop `Notify` call over D-Bus, so title and body never enter any argv (`ps`).
/// One live alert per room (`replaces_id`): the newest message replaces the previous one. A click
/// arrives as `ActionInvoked`, read by one listener thread that lives as long as the connection.
#[cfg(all(target_os = "linux", feature = "desktop"))]
mod linux {
    use super::*;
    use std::collections::HashMap;
    use zbus::{blocking::{Connection, Proxy}, zvariant::Value};

    type Ids = Arc<Mutex<HashMap<RoomScope, (u32, Option<AccountId>)>>>;
    pub struct Bus {
        proxy: Proxy<'static>,
        ids: Ids,
        markup: bool,
    }
    static BUS: Mutex<Option<Bus>> = Mutex::new(None);

    fn connect(click: &Click) -> Result<Bus, NotificationError> {
        let conn = Connection::session().map_err(|_| NotificationError::Unavailable)?;
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )
        .map_err(|_| NotificationError::Unavailable)?;
        let markup = proxy
            .call::<_, _, Vec<String>>("GetCapabilities", &())
            .map_err(|_| NotificationError::Unavailable)?
            .iter()
            .any(|c| c == "body-markup");
        let signals = proxy.receive_signal("ActionInvoked").map_err(|_| NotificationError::Unavailable)?;
        let ids = Ids::default();
        let (slot, known) = (click.clone(), ids.clone());
        std::thread::spawn(move || {
            for message in signals {
                let Ok((id, action)) = message.body().deserialize::<(u32, String)>() else { continue };
                if action != "open" {
                    continue;
                }
                let target = known.lock().ok().and_then(|ids| {
                    ids.iter().find(|(_, (known, _))| *known == id).map(|(scope, (_, account))| (*account, scope.clone()))
                });
                if let (Some(target), Ok(mut slot)) = (target, slot.lock()) {
                    *slot = Some(target);
                }
            }
        });
        Ok(Bus { proxy, ids, markup })
    }

    /// The daemon renders a small HTML subset: message text must not become markup or links.
    fn escape(body: &str) -> String {
        body.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn deliver(notification: Notification, click: &Click) -> Result<(), NotificationError> {
        let mut guard = BUS.lock().map_err(|_| NotificationError::Worker)?;
        if guard.is_none() {
            *guard = Some(connect(click)?);
        }
        let bus = guard.as_ref().expect("connected above");
        let (account, scope) = {
            let (account, scope) = notification.target();
            (account, scope.clone())
        };
        let replaces = bus.ids.lock().ok().and_then(|ids| ids.get(&scope).map(|(id, _)| *id)).unwrap_or(0);
        let body = notification.body();
        let body = if bus.markup { escape(&body) } else { body };
        let hints = HashMap::from([
            ("urgency", Value::from(0u8)),
            ("suppress-sound", Value::from(true)),
            ("transient", Value::from(true)),
            ("category", Value::from("im.received")),
        ]);
        let sent = bus.proxy.call::<_, _, u32>(
            "Notify",
            &("Gnobbi", replaces, "", notification.title(), body, vec!["open", "Apri"], hints, 5000i32),
        );
        match sent {
            Ok(id) => {
                if let Ok(mut ids) = bus.ids.lock() {
                    // shortcut: one id per room ever alerted; upgrade if rooms become unbounded.
                    ids.insert(scope, (id, account));
                }
                Ok(())
            }
            Err(_) => {
                *guard = None; // the daemon or the bus went away: reconnect on the next alert
                Err(NotificationError::Unavailable)
            }
        }
    }
}
#[cfg(all(target_os = "linux", feature = "desktop"))]
fn deliver_os(notification: Notification, click: &Click) -> Result<(), NotificationError> {
    linux::deliver(notification, click)
}
/// Fixed AppleScript, no placeholders: title and body reach it only through the environment of
/// the child (never argv, so never in `ps`, and never inside the script text). `system attribute`
/// decodes as MacRoman (emoji/accents garbled), so the fixed `printf` re-reads it as UTF-8.
#[cfg(any(target_os = "macos", test))]
const MACOS_SCRIPT: [&str; 2] = [
    "-e",
    "display notification (do shell script \"printf %s \\\"$GNOBBI_BODY\\\"\" as «class utf8») with title (do shell script \"printf %s \\\"$GNOBBI_TITLE\\\"\" as «class utf8»)",
];
#[cfg(any(target_os = "macos", test))]
fn macos_env(notification: &Notification) -> [(&'static str, String); 2] {
    [("GNOBBI_TITLE", notification.title().into()), ("GNOBBI_BODY", notification.body())]
}

/// Blocking platform boundary: only the notification worker may call this.
#[cfg(target_os = "macos")]
// shortcut: osascript alerts report no click, so on macOS they only name the account.
fn deliver_os(notification: Notification, _: &Click) -> Result<(), NotificationError> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("/usr/bin/osascript")
        .args(MACOS_SCRIPT)
        .envs(macos_env(&notification))
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
#[cfg(not(any(all(target_os = "linux", feature = "desktop"), target_os = "macos")))]
fn deliver_os(_: Notification, _: &Click) -> Result<(), NotificationError> {
    Err(NotificationError::Unavailable)
}

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
