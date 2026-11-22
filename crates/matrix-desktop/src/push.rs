//! iOS remote push, phase 1: the APNs device token feeds the Matrix pushers of every account (`matrix_engine::push`).
//! Phase 2 (Notification Service Extension: decrypted content, room opening on tap) is not here.
#[cfg(target_os = "ios")]
use crate::AppDirs;

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

#[cfg(target_os = "ios")]
mod ios {
    use super::*;
    use matrix_engine::push::{PushPlatform, PushSettings, set};
    use objc2::{AnyThread, MainThreadMarker, define_class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSObject, NSOperationQueue};
    use objc2_ui_kit::UIApplication;
    use std::sync::{Mutex, Once};

    static TOKEN: Mutex<Option<String>> = Mutex::new(None);
    static DIRS: Mutex<Option<AppDirs>> = Mutex::new(None);

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
                    received(super::token_hex(raw));
                }
            }
            #[unsafe(method(application:didFailToRegisterForRemoteNotificationsWithError:))]
            fn did_fail(&self, _: &AnyObject, _: &AnyObject) {
                eprintln!("Gnobbi: APNs registration failed");
            }
        }
    );

    /// Token (from APNs or from settings.json) + current language -> every account worker.
    fn publish() {
        let token = TOKEN.lock().ok().and_then(|t| t.clone());
        set(token.map(|token| PushSettings { platform: PushPlatform::Apns, token, lang: crate::i18n::current().to_owned(), body: crate::i18n::t("New message").to_owned() }));
    }

    fn received(token: String) {
        if TOKEN.lock().map(|mut t| t.replace(token.clone()).as_ref() != Some(&token)).unwrap_or(false) {
            if let Some(dirs) = DIRS.lock().ok().and_then(|d| d.clone()) {
                let _ = crate::preferences::update_settings(&dirs, |m| { m.insert("push_token".into(), token.into()); });
            }
            publish();
        }
    }

    /// At launch, once the language is applied: the last known token registers the pushers at once, APNs confirms it later.
    pub fn init(dirs: &AppDirs) {
        *DIRS.lock().unwrap_or_else(|e| e.into_inner()) = Some(dirs.clone());
        let saved = crate::preferences::read_settings(dirs).get("push_token").and_then(|v| v.as_str()).map(str::to_owned);
        *TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = saved;
        crate::i18n::on_change(|_| publish());
        publish();
    }

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
pub use ios::{init, register};

#[cfg(test)]
mod tests {
    #[test]
    fn token_is_lowercase_hex() {
        assert_eq!(super::token_hex(&[0x00, 0xAB, 0x0f, 0xff]), "00ab0fff");
        assert_eq!(super::token_hex(&[]), "");
        assert_eq!(super::token_class(&[0xAB; 32]), "32 bytes, 64 chars, lowercase hex");
        assert_eq!(super::token_class(&[]), "0 bytes, 0 chars, lowercase hex");
    }
}
