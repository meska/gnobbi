//! iOS remote push, phase 1: the APNs device token feeds the Matrix pushers of every account (`matrix_engine::push`).
//! Phase 2 (Notification Service Extension: decrypted content, room opening on tap) is not here.
use crate::AppDirs;
use matrix_engine::push::{PushPlatform, PushSettings, set};
use std::sync::Mutex;

/// APNs gives raw bytes; the pushkey is lowercase hex. Sygnal must run with `convert_device_token_to_hex: false`
/// (its default base64-decodes the pushkey, turns 64 hex chars into a 96-char token and APNs answers BadDeviceToken).
pub fn token_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Log-safe description of a token (never the token): byte count and pushkey format.
pub fn token_class(bytes: &[u8]) -> String {
    let hex = token_hex(bytes);
    let ok = hex.len() == 2 * bytes.len() && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    format!("{} bytes, {} chars, {}", bytes.len(), hex.len(), if ok { "lowercase hex" } else { "NOT lowercase hex" })
}

static TOKEN: Mutex<Option<String>> = Mutex::new(None);
static DIRS: Mutex<Option<AppDirs>> = Mutex::new(None);

/// Token (from the OS push service or from settings.json) + current language -> every account worker.
/// The engine keeps the last value in a watch, so a token that arrives before any worker exists is not lost.
fn publish() {
    let token = TOKEN.lock().ok().and_then(|t| t.clone());
    let platform = if cfg!(target_os = "ios") { PushPlatform::Apns } else { PushPlatform::Fcm };
    set(token.map(|token| PushSettings { platform, token, lang: crate::i18n::current().to_owned(), body: crate::i18n::t("New message").to_owned() }));
}

fn save(dirs: &AppDirs, token: &str) {
    let _ = crate::preferences::update_settings(dirs, |m| { m.insert("push_token".into(), token.into()); });
}

/// A new token (iOS: APNs; Android: first FCM token and every `onNewToken`). Callable before `init`: the token is kept and persisted then.
pub fn received(token: String) {
    if TOKEN.lock().map(|mut t| t.replace(token.clone()).as_ref() != Some(&token)).unwrap_or(false) {
        if let Some(dirs) = DIRS.lock().ok().and_then(|d| d.clone()) {
            save(&dirs, &token);
        }
        publish();
    }
}

/// At launch, once the language is applied: the last known token registers the pushers at once, the OS confirms it later.
/// A token that arrived first (Android: push service started the process) wins over the saved one.
pub fn init(dirs: &AppDirs) {
    *DIRS.lock().unwrap_or_else(|e| e.into_inner()) = Some(dirs.clone());
    let fresh = TOKEN.lock().ok().and_then(|t| t.clone());
    match fresh {
        Some(token) => save(dirs, &token),
        None => *TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = crate::preferences::read_settings(dirs).get("push_token").and_then(|v| v.as_str()).map(str::to_owned),
    }
    crate::i18n::on_change(|_| publish());
    publish();
}

/// JNI entry points of `net.meskatech.gnobbi.Native` (static natives, see packaging/android).
#[cfg(target_os = "android")]
mod android {
    use jni::{errors::LogErrorAndDefault, objects::{JClass, JString}, Env, EnvUnowned};

    fn text(env: &mut Env, s: &JString) -> jni::errors::Result<String> {
        s.try_to_string(env)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_meskatech_gnobbi_Native_nativeOnToken<'l>(mut env: EnvUnowned<'l>, _: JClass<'l>, token: JString<'l>) {
        env.with_env(|env| -> jni::errors::Result<()> {
            let token = text(env, &token)?;
            eprintln!("Gnobbi: FCM token received ({} chars)", token.len());
            super::received(token);
            Ok(())
        })
        .resolve::<LogErrorAndDefault>()
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_meskatech_gnobbi_Native_nativeOnOpen<'l>(mut env: EnvUnowned<'l>, _: JClass<'l>, account: JString<'l>, room: JString<'l>) {
        env.with_env(|env| -> jni::errors::Result<()> {
            let (account, room) = (text(env, &account)?, text(env, &room)?);
            eprintln!("Gnobbi: notification tap accepted={}", crate::notifications::open_from_push(&account, &room));
            Ok(())
        })
        .resolve::<LogErrorAndDefault>()
    }
}

#[cfg(target_os = "ios")]
mod ios {
    use objc2::{AnyThread, MainThreadMarker, define_class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSObject, NSOperationQueue};
    use objc2_ui_kit::UIApplication;
    use std::sync::Once;

    define_class!(
        // winit passes no UIApplicationDelegate to UIApplicationMain, so ours is the only one; UIKit only needs the selectors.
        #[unsafe(super(NSObject))]
        #[name = "GnobbiPushDelegate"]
        #[ivars = ()]
        struct PushDelegate;
        impl PushDelegate {
            #[unsafe(method(application:didRegisterForRemoteNotificationsWithDeviceToken:))]
            fn did_register(&self, _: &AnyObject, token: &AnyObject) {
                let (len, bytes): (usize, *const u8) = unsafe { (msg_send![token, length], msg_send![token, bytes]) };
                if !bytes.is_null() {
                    // SAFETY: NSData owns `len` bytes at `bytes` for the duration of this call.
                    let raw = unsafe { std::slice::from_raw_parts(bytes, len) };
                    eprintln!("Gnobbi: APNs token received ({})", super::token_class(raw));
                    super::received(super::token_hex(raw));
                }
            }
            #[unsafe(method(application:didFailToRegisterForRemoteNotificationsWithError:))]
            fn did_fail(&self, _: &AnyObject, _: &AnyObject) {
                eprintln!("Gnobbi: APNs registration failed");
            }
        }
    );

    /// Main thread, after the notification permission was granted: install the delegate and ask APNs for the token.
    pub fn register() {
        // SAFETY: the block only runs on the main queue and captures nothing.
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block2::RcBlock::new(|| {
            static ONCE: Once = Once::new();
            let Some(mtm) = MainThreadMarker::new() else { return };
            let app = UIApplication::sharedApplication(mtm);
            ONCE.call_once(|| {
                let delegate: Retained<PushDelegate> = msg_send![super(PushDelegate::alloc().set_ivars(())), init];
                let _: () = msg_send![&*app, setDelegate: &*delegate];
                std::mem::forget(delegate); // UIApplication keeps only a weak reference
            });
            let _: () = msg_send![&*app, registerForRemoteNotifications];
        })) };
    }
}
#[cfg(target_os = "ios")]
pub use ios::register;

#[cfg(test)]
mod tests {
    use super::*;

    /// One test: the token and the engine watch are process globals. Android order: the push service delivers the token
    /// before the app (and its dirs) exists; a tap-less cold start must still end up with the fresh token persisted.
    #[test]
    fn token_received_before_init_is_published_then_persisted_and_beats_the_saved_one() {
        let dirs = AppDirs { config: std::env::temp_dir().join(format!("gnobbi-push-{}", std::process::id())), data: Default::default(), cache: Default::default() };
        std::fs::create_dir_all(&dirs.config).unwrap();
        save(&dirs, "old-saved-token");
        received("fcm-fake-token".into());
        let now = matrix_engine::push::current().expect("published without any worker or dirs");
        assert_eq!((now.platform, now.token.as_str()), (PushPlatform::Fcm, "fcm-fake-token"));
        init(&dirs);
        assert_eq!(matrix_engine::push::current().unwrap().token, "fcm-fake-token");
        assert_eq!(crate::preferences::read_settings(&dirs).get("push_token").and_then(|v| v.as_str()), Some("fcm-fake-token"));
        received("fcm-rotated".into());
        assert_eq!(matrix_engine::push::current().unwrap().token, "fcm-rotated");
        assert_eq!(crate::preferences::read_settings(&dirs).get("push_token").and_then(|v| v.as_str()), Some("fcm-rotated"));
        let _ = std::fs::remove_dir_all(&dirs.config);
    }

    #[test]
    fn token_is_lowercase_hex() {
        assert_eq!(super::token_hex(&[0x00, 0xAB, 0x0f, 0xff]), "00ab0fff");
        assert_eq!(super::token_hex(&[]), "");
        assert_eq!(super::token_class(&[0xAB; 32]), "32 bytes, 64 chars, lowercase hex");
        assert_eq!(super::token_class(&[]), "0 bytes, 0 chars, lowercase hex");
    }
}
