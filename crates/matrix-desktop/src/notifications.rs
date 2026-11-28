//! Bounded local OS notification adapter, separate from the Matrix runtime.
use matrix_domain::{AccountId, RoomScope, notifications::Notification};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use crate::notification_avatar::{self as avatar, Avatar};
use matrix_engine::avatar::AvatarPicture;
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
    sender: mpsc::SyncSender<(u64, Notification, Option<mpsc::Receiver<AvatarPicture>>)>,
    revision: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
}
impl NotificationHandle {
    /// Nonblocking, best effort: one pending alert maximum.
    pub fn submit(&self, notification: Notification) -> bool {
        self.submit_with_avatar(notification, None)
    }
    /// Like `submit`; the worker waits at most `avatar::WAIT` for the picture, off the caller's thread.
    pub fn submit_with_avatar(&self, notification: Notification, picture: Option<mpsc::Receiver<AvatarPicture>>) -> bool {
        self.sender
            .try_send((self.revision.load(Ordering::Acquire), notification, picture))
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
        init();
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if let Some(dir) = avatar::dir() {
            avatar::sweep(&dir, Duration::from_secs(24 * 3600));
        }
        let click = Click::default();
        let slot = click.clone();
        Self::spawn_clicks(click, move |n, a| deliver_os(n, a, &slot))
    }
    #[cfg(test)]
    fn spawn_with<F>(deliver: F) -> std::io::Result<Self>
    where
        F: FnMut(Notification, Option<Avatar>) -> Result<(), NotificationError> + Send + 'static,
    {
        Self::spawn_clicks(Click::default(), deliver)
    }
    fn spawn_clicks<F>(click: Click, mut deliver: F) -> std::io::Result<Self>
    where
        F: FnMut(Notification, Option<Avatar>) -> Result<(), NotificationError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel::<(u64, Notification, Option<mpsc::Receiver<AvatarPicture>>)>(1);
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
                        Ok((version, notification, picture)) => {
                            if !cancelled.load(Ordering::Acquire)
                                && version == current.load(Ordering::Acquire)
                            {
                                let picture = picture.and_then(|rx| avatar::wait(&rx, avatar::WAIT));
                                failure.store(deliver(notification, picture).is_err(), Ordering::Release);
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
#[cfg(target_os = "linux")]
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

    pub fn deliver(notification: Notification, picture: Option<Avatar>, click: &Click) -> Result<(), NotificationError> {
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
        let mut hints = HashMap::from([
            ("urgency", Value::from(0u8)),
            ("suppress-sound", Value::from(true)),
            ("transient", Value::from(true)),
            ("category", Value::from("im.received")),
        ]);
        if let Some(a) = picture {
            // freedesktop `image-data`: (width, height, rowstride, has_alpha, bits, channels, bytes)
            hints.insert("image-data", Value::from((a.width as i32, a.height as i32, a.width as i32 * 4, true, 8i32, 4i32, a.rgba)));
        }
        let sent = bus.proxy.call::<_, _, u32>(
            "Notify",
            &("Gnobbi", replaces, "", notification.title_line(), body, vec!["open", crate::i18n::t("Open")], hints, 5000i32),
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
#[cfg(target_os = "linux")]
fn deliver_os(notification: Notification, picture: Option<Avatar>, click: &Click) -> Result<(), NotificationError> {
    linux::deliver(notification, picture, click)
}
/// What an alert carries so a click can be understood by a process that did not raise it (the app was
/// closed and the click launched it): the account and the room, as `<account>:<room id>`. Also the OS
/// request identifier, so one room keeps one live alert across restarts. Worker ids repeat between
/// profile folders, so the catalog id (0: unknown) comes first: `<catalog>:<worker>:<room id>`.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android", test))]
fn target_key(catalog: Option<AccountId>, scope: &RoomScope) -> String {
    format!("{}:{}:{}", catalog.map_or(0, |a| a.as_u64()), scope.account_id.as_u64(), scope.room_id.as_str())
}
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android", test))]
fn parse_target(key: &str) -> Option<(Option<AccountId>, RoomScope)> {
    let (catalog, rest) = key.split_once(':')?;
    let (account, room) = rest.split_once(':')?;
    let catalog = catalog.parse::<u64>().ok()?;
    let scope = RoomScope { account_id: AccountId::new(account.parse().ok()?), room_id: matrix_domain::RoomId::new(room) };
    Some(((catalog != 0).then(|| AccountId::new(catalog)), scope))
}

/// A tap on a remote push the extension did not rewrite: the payload carries `gnobbi_account` (`<catalog>:<worker>:<dir>`,
/// see `matrix_engine::push`) and Sygnal's `room_id`.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android", test))]
fn parse_remote(account: &str, room: &str) -> Option<(Option<AccountId>, RoomScope)> {
    let mut parts = account.splitn(3, ':');
    let (catalog, worker) = (parts.next()?, parts.next()?);
    if !room.starts_with('!') || catalog.parse::<u64>().ok()? == 0 {
        return None;
    }
    parse_target(&format!("{catalog}:{worker}:{room}"))
}

/// Identifiers of the delivered notifications (`(identifier, thread identifier, gnobbi key)`) that belong to `scope`.
/// The key (`<catalog>:<worker>:<room>`, empty if absent) is on our alerts and on pushes the extension rewrote; it names the
/// account, so another account's alert in the same room stays. Without a key a push is matched by its thread id (the
/// extension sets it to the room), and an old local alert by its identifier (the same text). An explicit thread id that
/// names another room always wins. An unknown catalog id (None or 0) on either side matches any.
#[cfg(any(target_os = "macos", target_os = "ios", test))]
fn delivered_to_remove<'a>(delivered: &'a [(String, String, String)], catalog: Option<AccountId>, scope: &RoomScope) -> Vec<&'a str> {
    let room = scope.room_id.as_str();
    if room.is_empty() {
        return Vec::new();
    }
    let belongs = |id: &str, thread: &str, key: &str| {
        if !thread.is_empty() && thread != room {
            return false;
        }
        match parse_target(if key.is_empty() { id } else { key }) {
            Some((owner, found)) => found == *scope && (owner.is_none() || catalog.is_none() || owner == catalog),
            None => thread == room,
        }
    };
    delivered.iter().filter(|(id, thread, key)| belongs(id, thread, key)).map(|(id, _, _)| id.as_str()).collect()
}

/// Rooms whose delivered alerts must go. `rooms`: `(scope, unread)` as the sidebar shows them; `alerted`: rooms seen unread
/// (a room back at zero is read here or elsewhere). A zero never seen above zero is not enough: it may be the cache's.
pub fn alerts_to_clear(rooms: &[(&RoomScope, u64)], alerted: &mut std::collections::HashSet<String>) -> Vec<RoomScope> {
    let mut clear = Vec::new();
    for (scope, unread) in rooms {
        let id = scope.room_id.as_str();
        if *unread > 0 {
            alerted.insert(id.to_owned());
        } else if alerted.remove(id) {
            clear.push((*scope).clone());
        }
    }
    clear
}

/// These rooms were read (here or on another device): their alerts leave the Notification Center, with one read of the
/// delivered list for all of them. `catalog` is the catalog id of the account that read them (None: unknown). Best effort, async.
pub fn clear_rooms(catalog: Option<AccountId>, rooms: Vec<RoomScope>) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    macos::clear_rooms(catalog, rooms);
    #[cfg(target_os = "android")]
    android::clear_rooms(catalog, rooms);
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
    let _ = (catalog, rooms);
}

/// Back from the background: the next `set_badge` writes the total even if unchanged (a push may have set the badge meanwhile).
pub fn reset_badge() {
    #[cfg(target_os = "ios")]
    macos::reset_badge();
}

/// The app icon's badge: unread messages over all accounts. iOS only (macOS keeps its dock tile untouched).
/// `active`: the app is in the foreground; coming back to it rewrites the badge, since a push may have set it meanwhile.
pub fn set_badge(unread: u64, active: bool) {
    #[cfg(target_os = "ios")]
    macos::set_badge(unread, active);
    #[cfg(not(target_os = "ios"))]
    let _ = (unread, active);
}

/// Where a tap lands: the room alone, or its thread panel opened on `root` with the notified `event` in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TapRoute {
    Chat(RoomScope),
    Thread { scope: RoomScope, root: String, event: String },
}

/// The push's `thread_root` / `event_id` of the last tap, with its room: read by the session that opens the room.
#[cfg(any(target_os = "macos", target_os = "ios", test))]
static THREAD_TAP: Mutex<Option<(String, String, String)>> = Mutex::new(None);

/// A tap's `userInfo` on a push inside a thread. Both ids must be event ids (`$...`); anything else is a plain chat tap.
#[cfg(any(target_os = "macos", target_os = "ios", test))]
fn remember_thread_tap(room: &str, root: Option<&str>, event: Option<&str>) {
    let hint = root.zip(event).filter(|(r, e)| r.starts_with('$') && e.starts_with('$')).map(|(r, e)| (room.to_owned(), r.to_owned(), e.to_owned()));
    *THREAD_TAP.lock().unwrap_or_else(|e| e.into_inner()) = hint;
}

/// The route for a tap that opens `scope`; consumes the thread hint when it names that room.
pub fn route_tap(scope: RoomScope) -> TapRoute {
    #[cfg(any(target_os = "macos", target_os = "ios", test))]
    if let Ok(mut hint) = THREAD_TAP.lock()
        && let Some((_, root, event)) = hint.take_if(|(room, ..)| room == scope.room_id.as_str())
    {
        return TapRoute::Thread { scope, root, event };
    }
    TapRoute::Chat(scope)
}

/// A click that reached the process before (or without) the session that owns it: kept here until that account polls.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android", test))]
static EARLY_CLICK: Mutex<Option<(Option<AccountId>, RoomScope)>> = Mutex::new(None);

/// The alert clicked while no live session knew it (a click that launched the app), if it belongs to the
/// catalog account `account`. An alert with no catalog id (unknown) goes to the first asker.
pub fn take_early_click(account: Option<AccountId>) -> Option<RoomScope> {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android", test))]
    {
        let mut early = EARLY_CLICK.lock().ok()?;
        return early.take_if(|(owner, _)| owner.is_none() || *owner == account).map(|(_, scope)| scope);
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android", test)))]
    {
        let _ = account;
        None
    }
}

/// The room on screen in a focused window (catalog account, room), as the shown session last said; None when nothing is.
#[cfg(any(target_os = "android", test))]
static VISIBLE: Mutex<Option<(Option<AccountId>, RoomScope)>> = Mutex::new(None);

/// Called every frame by the shown session (None: hidden, suspended or no room). Only Android reads it, for pushes.
pub fn set_visible(room: Option<(Option<AccountId>, RoomScope)>) {
    #[cfg(any(target_os = "android", test))]
    {
        *VISIBLE.lock().unwrap_or_else(|e| e.into_inner()) = room;
    }
    #[cfg(not(any(target_os = "android", test)))]
    let _ = room;
}

/// True when the push for `account` (`<catalog>:<worker>:<dir>`) and `room` is for the room the user is reading right now.
/// A malformed pair is not visible: the notification is shown.
#[cfg(any(target_os = "android", test))]
pub fn is_visible(account: &str, room: &str) -> bool {
    let Some((catalog, scope)) = parse_remote(account, room) else { return false };
    let visible = VISIBLE.lock().unwrap_or_else(|e| e.into_inner());
    visible.as_ref().is_some_and(|(c, s)| *c == catalog && reading(true, Some(s), &scope))
}

/// Android: the delivered alerts are posted by the FCM service with tag `target_key` (same string as on Apple) and id 0.
#[cfg(target_os = "android")]
mod android {
    use super::*;
    use jni::{jni_sig, jni_str, objects::JObject, JValue};

    pub fn clear_rooms(catalog: Option<AccountId>, rooms: Vec<RoomScope>) {
        // Without the catalog id the tag cannot be built: the alert stays until its room is read again.
        if rooms.is_empty() || catalog.is_none() {
            return;
        }
        let Ok(ctx) = std::panic::catch_unwind(ndk_context::android_context) else { return };
        // SAFETY: the VM pointer comes from android-activity; from_raw is idempotent.
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
        let res = vm.attach_current_thread(|env| -> Result<(), jni::errors::Error> {
            // SAFETY: the context is a global ref owned by android-activity; JObject has no Drop and is not kept.
            let context = unsafe { JObject::from_raw(env, ctx.context().cast()) };
            let service = env.new_string("notification")?;
            let nm = env
                .call_method(&context, jni_str!("getSystemService"), jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"), &[JValue::from(&service)])?
                .l()?;
            for scope in &rooms {
                let tag = env.new_string(target_key(catalog, scope))?;
                env.call_method(&nm, jni_str!("cancel"), jni_sig!("(Ljava/lang/String;I)V"), &[JValue::from(&tag), JValue::Int(0)])?;
            }
            Ok(())
        });
        if let Err(e) = res {
            log::warn!("gnobbi: clearing notifications failed: {e:?}");
        }
    }
}

/// Android: a tap on a local notification reaches the process as intent extras (`gnobbi_account`, `room_id`), also when the
/// app was closed. Same slot and validation as an iOS tap on a remote push; false when the extras are malformed.
#[cfg(any(target_os = "android", test))]
pub fn open_from_push(account: &str, room: &str) -> bool {
    let Some(target) = parse_remote(account, room) else { return false };
    *EARLY_CLICK.lock().unwrap_or_else(|e| e.into_inner()) = Some(target);
    true
}

/// True (and the click dropped) when the pending early click names a catalog account that is not in `known`.
pub fn drop_early_click_of_gone_account(known: &[AccountId]) -> bool {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android", test))]
    if let Ok(mut early) = EARLY_CLICK.lock() {
        return early.take_if(|(owner, _)| owner.is_some_and(|o| !known.contains(&o))).is_some();
    }
    let _ = known;
    false
}

/// Call at process start: macOS hands a click that launched the app to the delegate, which must exist by then.
pub fn init() {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    macos::install_delegate();
}

/// macOS: UNUserNotificationCenter (the AppKit/NSUserNotification path is deprecated). One live alert
/// per room (same request identifier replaces the previous one); a click reaches the delegate, which
/// fills the `Click` slot of the session that raised it (as on Linux) or, when that session does not
/// exist (the click launched the app), `EARLY_CLICK`. Needs a bundled, signed app: without a bundle id
/// (`cargo run`) the center would abort the process, so the alert is reported unavailable instead.
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod macos {
    use super::*;
    use block2::{DynBlock, RcBlock};
    use objc2::{AnyThread, define_class, msg_send, rc::Retained, runtime::ProtocolObject};
    use objc2_foundation::{NSArray, NSBundle, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString, NSURL};
    use objc2_user_notifications::*;
    use std::collections::HashMap;
    use std::sync::Once;

    const KEY: &str = "gnobbi";
    // Each room maps to the `Click` slot of the worker (= account session) that alerted: the delegate is one per process.
    static SLOTS: Mutex<Option<HashMap<String, Click>>> = Mutex::new(None);

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "GnobbiNotificationDelegate"]
        #[ivars = ()]
        struct Delegate;
        unsafe impl NSObjectProtocol for Delegate {}
        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(&self, _: &UNUserNotificationCenter, response: &UNNotificationResponse, done: &DynBlock<dyn Fn()>) {
                let info = response.notification().request().content().userInfo();
                let text = |name: &str| info.objectForKey(&NSString::from_str(name)).and_then(|v| v.downcast::<NSString>().ok()).map(|k| k.to_string());
                let key = text(KEY);
                let target = match &key {
                    Some(key) => parse_target(key),
                    None => text("gnobbi_account").zip(text("room_id")).and_then(|(account, room)| parse_remote(&account, &room)),
                };
                if let Some((catalog, scope)) = target {
                    remember_thread_tap(scope.room_id.as_str(), text("thread_root").as_deref(), text("event_id").as_deref());
                    let key = target_key(catalog, &scope);
                    let slot = SLOTS.lock().ok().and_then(|slots| slots.as_ref()?.get(&key).cloned());
                    match slot.as_ref().map(|slot| slot.lock()) {
                        Some(Ok(mut slot)) => *slot = Some((catalog, scope)),
                        _ => *EARLY_CLICK.lock().unwrap_or_else(|e| e.into_inner()) = Some((catalog, scope)),
                    }
                }
                done.call(());
            }
            // The window decides upstream (`reading`) whether an alert is wanted: when it is, show it even in the foreground.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(&self, _: &UNUserNotificationCenter, notification: &UNNotification, done: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
                // A remote push (it has a push trigger; our own alerts have none) in the foreground duplicates the alert
                // the sync raises itself, or shows a room the user is looking at: drop it, extension-rewritten or not.
                #[cfg(target_os = "ios")]
                if notification.request().trigger().is_some() {
                    done.call((UNNotificationPresentationOptions::empty(),));
                    return;
                }
                #[cfg(not(target_os = "ios"))]
                let _ = notification;
                // iOS plays the sound only if the foreground presentation asks for it.
                #[cfg(target_os = "ios")]
                let options = UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List | UNNotificationPresentationOptions::Sound;
                #[cfg(not(target_os = "ios"))]
                let options = UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List;
                done.call((options,));
            }
        }
    );

    pub fn install_delegate() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            if NSBundle::mainBundle().bundleIdentifier().is_none() {
                return;
            }
            let delegate: Retained<Delegate> = unsafe { msg_send![super(Delegate::alloc().set_ivars(())), init] };
            UNUserNotificationCenter::currentNotificationCenter().setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            std::mem::forget(delegate); // the center keeps only a weak reference
            // iOS: ask at launch, so the prompt is not first raised by (and the alert lost with) the first incoming message.
            #[cfg(target_os = "ios")]
            authorize();
        });
    }

    fn authorize() {
        // Simulator-only test hook; release builds always request the normal permission.
        #[cfg(all(target_os = "ios", debug_assertions))]
        if std::env::var_os("GNOBBI_DEBUG_SKIP_NOTIFICATION_PROMPT").is_some() { return; }
        static AUTH: Once = Once::new();
        AUTH.call_once(|| UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge, &RcBlock::new(|granted: objc2::runtime::Bool, error: *mut NSError| {
            // An error is not a grant, whatever `granted` says.
            let granted = granted.as_bool() && error.is_null();
            // Remote push needs the permission first; every launch asks APNs again, the token may have changed.
            #[cfg(target_os = "ios")]
            if granted {
                crate::push::permission_granted();
                crate::push::register();
                RESET_BADGE.store(true, Ordering::Relaxed); // the next tick writes the total: the badge was refused until now
            }
            #[cfg(not(target_os = "ios"))]
            let _ = granted;
        })));
    }

    pub fn clear_rooms(catalog: Option<AccountId>, rooms: Vec<RoomScope>) {
        if rooms.is_empty() || NSBundle::mainBundle().bundleIdentifier().is_none() {
            return;
        }
        UNUserNotificationCenter::currentNotificationCenter().getDeliveredNotificationsWithCompletionHandler(&RcBlock::new(move |list: std::ptr::NonNull<NSArray<UNNotification>>| {
            // SAFETY: the center passes a live array for the duration of the callback.
            let delivered: Vec<(String, String, String)> = unsafe { list.as_ref() }.iter().map(|n| {
                let content = n.request().content();
                let key = content.userInfo().objectForKey(&NSString::from_str(KEY)).and_then(|v| v.downcast::<NSString>().ok()).map(|k| k.to_string());
                (n.request().identifier().to_string(), content.threadIdentifier().to_string(), key.unwrap_or_default())
            }).collect();
            let ids: Vec<Retained<NSString>> = rooms.iter().flat_map(|room| delivered_to_remove(&delivered, catalog, room)).map(NSString::from_str).collect();
            if !ids.is_empty() {
                UNUserNotificationCenter::currentNotificationCenter().removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(&ids));
            }
        }));
    }

    /// Set when the permission (Badge included) was just granted: the cached total may never have reached the icon.
    #[cfg(target_os = "ios")]
    static RESET_BADGE: AtomicBool = AtomicBool::new(false);

    #[cfg(target_os = "ios")]
    pub fn reset_badge() {
        RESET_BADGE.store(true, Ordering::Relaxed);
    }

    #[cfg(target_os = "ios")]
    pub fn set_badge(unread: u64, active: bool) {
        static LAST: AtomicU64 = AtomicU64::new(u64::MAX);
        static WAS_ACTIVE: AtomicBool = AtomicBool::new(false);
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            return;
        }
        // Back in the foreground (or just authorized): APNs may have written the badge meanwhile, so write it even if the total is unchanged.
        if (active && !WAS_ACTIVE.swap(true, Ordering::Relaxed)) || RESET_BADGE.swap(false, Ordering::Relaxed) {
            LAST.store(u64::MAX, Ordering::Relaxed);
        }
        if !active {
            WAS_ACTIVE.store(false, Ordering::Relaxed);
        }
        if LAST.swap(unread, Ordering::Relaxed) == unread {
            return;
        }
        // The app's deployment target is iOS 17: setBadgeCount (16+) needs no fallback.
        UNUserNotificationCenter::currentNotificationCenter().setBadgeCount_withCompletionHandler(unread.min(i32::MAX as u64) as isize, None);
    }

    pub fn deliver(notification: Notification, picture: Option<Avatar>, click: &Click) -> Result<(), NotificationError> {
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            return Err(NotificationError::Unavailable);
        }
        install_delegate();
        let center = UNUserNotificationCenter::currentNotificationCenter();
        authorize();
        let (catalog, scope) = notification.target();
        let key_text = target_key(catalog, scope);
        let key = NSString::from_str(&key_text);
        // shortcut: one slot per room ever alerted; upgrade if rooms become unbounded.
        SLOTS.lock().map_err(|_| NotificationError::Worker)?.get_or_insert_with(HashMap::new).insert(key_text, click.clone()); // a re-added account has a new worker
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(notification.title()));
        if let Some(room) = notification.subtitle() {
            content.setSubtitle(&NSString::from_str(room));
        }
        content.setBody(&NSString::from_str(&notification.body()));
        content.setThreadIdentifier(&NSString::from_str(scope.room_id.as_str())); // groups by room, and lets a read match the exact thread
        #[cfg(target_os = "ios")]
        content.setSound(Some(&objc2_user_notifications::UNNotificationSound::defaultSound()));
        let info = NSDictionary::from_retained_objects(&[&*NSString::from_str(KEY)], &[key.clone()]);
        // SAFETY: the setter's untyped dictionary is the same object; only the generic parameters differ.
        unsafe { content.setUserInfo(&Retained::cast_unchecked::<NSDictionary>(info)) };
        // Thumbnail attachment (the left icon of a notification is the app's, only Communication Notifications can replace it).
        // The center copies the file when the request is added, so ours goes away right after.
        let file = picture.as_ref().and_then(avatar::png).zip(avatar::dir()).and_then(|(png, dir)| avatar::write(&dir, &png));
        if let Some(path) = &file
            && let Ok(attachment) = unsafe {
                UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(&NSString::from_str("avatar"), &NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())), None)
            }
        {
            content.setAttachments(&NSArray::from_retained_slice(&[attachment]));
        }
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&key, &content, None);
        let (done, result) = mpsc::channel();
        center.addNotificationRequest_withCompletionHandler(&request, Some(&RcBlock::new(move |e: *mut NSError| {
            let _ = done.send(e.is_null());
        })));
        let outcome = result.recv_timeout(Duration::from_secs(2));
        if let Some(path) = file {
            let _ = std::fs::remove_file(path);
        }
        match outcome {
            Ok(true) => Ok(()),
            Ok(false) => Err(NotificationError::Unavailable), // not authorized (yet) or refused by the system
            Err(_) => Err(NotificationError::Timeout),
        }
    }
}
/// Blocking platform boundary: only the notification worker may call this.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn deliver_os(notification: Notification, picture: Option<Avatar>, click: &Click) -> Result<(), NotificationError> {
    macos::deliver(notification, picture, click)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios")))]
fn deliver_os(_: Notification, _: Option<Avatar>, _: &Click) -> Result<(), NotificationError> {
    Err(NotificationError::Unavailable)
}

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
