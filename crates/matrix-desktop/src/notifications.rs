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
#[cfg(all(target_os = "linux", feature = "tray"))]
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
#[cfg(all(target_os = "linux", feature = "tray"))]
fn deliver_os(notification: Notification, click: &Click) -> Result<(), NotificationError> {
    linux::deliver(notification, click)
}
/// macOS: UNUserNotificationCenter (the AppKit/NSUserNotification path is deprecated). One live alert
/// per room (same request identifier replaces the previous one); a click reaches the delegate, which
/// fills the same `Click` slot as on Linux. Needs a bundled, signed app: without a bundle id
/// (`cargo run`) the center would abort the process, so the alert is reported unavailable instead.
#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use block2::{DynBlock, RcBlock};
    use objc2::{AnyThread, DefinedClass, define_class, msg_send, rc::Retained, runtime::ProtocolObject};
    use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
    use objc2_user_notifications::*;
    use std::collections::HashMap;

    // Each entry carries the `Click` slot of the worker (= account session) that alerted: the delegate is one per process.
    type Ids = HashMap<RoomScope, (u32, Option<AccountId>, Click)>;
    static IDS: Mutex<Option<Ids>> = Mutex::new(None);

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "GnobbiNotificationDelegate"]
        #[ivars = ()]
        struct Delegate;
        unsafe impl NSObjectProtocol for Delegate {}
        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(&self, _: &UNUserNotificationCenter, response: &UNNotificationResponse, done: &DynBlock<dyn Fn()>) {
                let id = response.notification().request().identifier().to_string();
                let target = IDS.lock().ok().and_then(|ids| {
                    ids.as_ref()?.iter().find(|(_, (n, ..))| n.to_string() == id).map(|(scope, (_, account, slot))| (slot.clone(), (*account, scope.clone())))
                });
                if let Some((slot, target)) = target && let Ok(mut slot) = slot.lock() {
                    *slot = Some(target);
                }
                done.call(());
            }
            // The window decides upstream (`reading`) whether an alert is wanted: when it is, show it even in the foreground.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(&self, _: &UNUserNotificationCenter, _: &UNNotification, done: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
                done.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
            }
        }
    );

    pub fn deliver(notification: Notification, click: &Click) -> Result<(), NotificationError> {
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            return Err(NotificationError::Unavailable);
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let mut guard = IDS.lock().map_err(|_| NotificationError::Worker)?;
        let ids = guard.get_or_insert_with(|| {
            let delegate: Retained<Delegate> = unsafe { msg_send![super(Delegate::alloc().set_ivars(())), init] };
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            std::mem::forget(delegate); // the center keeps only a weak reference
            center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &RcBlock::new(|_, _: *mut NSError| {}));
            Ids::default()
        });
        let (account, scope) = {
            let (account, scope) = notification.target();
            (account, scope.clone())
        };
        // shortcut: one id per room ever alerted; upgrade if rooms become unbounded.
        let next = ids.len() as u32 + 1;
        let entry = ids.entry(scope).or_insert((next, account, click.clone()));
        entry.2 = click.clone(); // a re-added account has a new worker
        let id = entry.0;
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(notification.title()));
        content.setBody(&NSString::from_str(&notification.body()));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id.to_string()), &content, None);
        let (done, result) = mpsc::channel();
        center.addNotificationRequest_withCompletionHandler(&request, Some(&RcBlock::new(move |e: *mut NSError| {
            let _ = done.send(e.is_null());
        })));
        drop(guard);
        match result.recv_timeout(Duration::from_secs(2)) {
            Ok(true) => Ok(()),
            Ok(false) => Err(NotificationError::Unavailable), // not authorized (yet) or refused by the system
            Err(_) => Err(NotificationError::Timeout),
        }
    }
}
/// Blocking platform boundary: only the notification worker may call this.
#[cfg(target_os = "macos")]
fn deliver_os(notification: Notification, click: &Click) -> Result<(), NotificationError> {
    macos::deliver(notification, click)
}
#[cfg(not(any(all(target_os = "linux", feature = "tray"), target_os = "macos")))]
fn deliver_os(_: Notification, _: &Click) -> Result<(), NotificationError> {
    Err(NotificationError::Unavailable)
}

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
