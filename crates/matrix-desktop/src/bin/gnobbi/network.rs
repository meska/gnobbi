//! The network in use, for the auto download: a phone tells a metered network (mobile data, a hotspot) from an unmetered one;
//! the desktop has one group of settings, which is the Wi-Fi one. A phone that cannot tell counts as being on mobile data.
use matrix_desktop::auto_download::Network;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn current() -> Network {
    Network::Wifi
}

/// Asked at most every 5 s: the answer is a system call (Android) and rows ask every frame.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub fn current() -> Network {
    static LAST: std::sync::Mutex<Option<(std::time::Instant, Network)>> = std::sync::Mutex::new(None);
    cached(&mut LAST.lock().unwrap_or_else(|e| e.into_inner()), platform_metered)
}

/// The cache keeps only a known answer: an unknown one (the iOS monitor has not reported yet) is asked again at the next frame.
#[cfg_attr(not(any(test, target_os = "ios", target_os = "android")), allow(dead_code))]
fn cached(last: &mut Option<(std::time::Instant, Network)>, ask: impl FnOnce() -> Option<bool>) -> Network {
    if let Some((at, net)) = *last && at.elapsed() < std::time::Duration::from_secs(5) {
        return net;
    }
    let metered = ask();
    let net = network_of(metered);
    *last = metered.map(|_| (std::time::Instant::now(), net));
    net
}

/// Unknown is mobile data.
#[cfg_attr(not(any(test, target_os = "ios", target_os = "android")), allow(dead_code))]
pub fn network_of(metered: Option<bool>) -> Network {
    if metered == Some(false) { Network::Wifi } else { Network::Mobile }
}

/// `ConnectivityManager.isActiveNetworkMetered`, through the activity.
#[cfg(target_os = "android")]
fn platform_metered() -> Option<bool> {
    use jni::{jni_sig, jni_str, objects::JObject};
    let (Ok(ctx), Some(app)) = (std::panic::catch_unwind(ndk_context::android_context), crate::ANDROID_APP.get()) else { return None };
    // SAFETY: as in pick::share_android.
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
    let res = vm.attach_current_thread(|env| -> Result<bool, jni::errors::Error> {
        // SAFETY: the activity is a global ref owned by android-activity; JObject has no Drop and is not kept.
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        let service = env.new_string("connectivity")?;
        let manager = env.call_method(&activity, jni_str!("getSystemService"), jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"), &[(&service).into()])?.l()?;
        env.call_method(&manager, jni_str!("isActiveNetworkMetered"), jni_sig!("()Z"), &[])?.z()
    });
    if let Err(e) = &res {
        log::warn!("gnobbi network: {e:?}");
    }
    res.ok()
}

/// `NWPathMonitor.isExpensive`: the monitor starts on the first question and keeps the last answer; until it has one, unknown.
#[cfg(target_os = "ios")]
fn platform_metered() -> Option<bool> {
    use std::ffi::{c_char, c_void};
    use std::sync::{OnceLock, atomic::{AtomicU8, Ordering}};
    // 0 unknown, 1 not expensive, 2 expensive.
    static STATE: AtomicU8 = AtomicU8::new(0);
    static MONITOR: OnceLock<usize> = OnceLock::new();
    #[link(name = "Network", kind = "framework")]
    unsafe extern "C" {
        fn nw_path_monitor_create() -> *mut c_void;
        fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
        fn nw_path_monitor_set_update_handler(monitor: *mut c_void, handler: &block2::Block<dyn Fn(*mut c_void)>);
        fn nw_path_monitor_start(monitor: *mut c_void);
        fn nw_path_is_expensive(path: *mut c_void) -> bool;
        fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut c_void;
    }
    MONITOR.get_or_init(|| {
        // SAFETY: the Network framework C API; the monitor and its queue live for the whole run (never released), the handler block
        // is copied by the framework and only stores one byte.
        unsafe {
            let monitor = nw_path_monitor_create();
            let queue = dispatch_queue_create(c"net.meskatech.gnobbi.network".as_ptr(), std::ptr::null_mut());
            let handler = block2::RcBlock::new(|path: *mut c_void| {
                STATE.store(if nw_path_is_expensive(path) { 2 } else { 1 }, Ordering::Relaxed);
            });
            nw_path_monitor_set_update_handler(monitor, &handler);
            nw_path_monitor_set_queue(monitor, queue);
            nw_path_monitor_start(monitor);
            monitor as usize
        }
    });
    match STATE.load(Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_network_known_not_to_be_metered_is_wifi() {
        assert_eq!(network_of(Some(false)), Network::Wifi);
        assert_eq!(network_of(Some(true)), Network::Mobile);
        assert_eq!(network_of(None), Network::Mobile);
    }

    #[test]
    fn the_cache_keeps_a_known_answer_and_asks_again_after_an_unknown_one() {
        let mut last = None;
        assert_eq!(cached(&mut last, || None), Network::Mobile);
        assert!(last.is_none(), "unknown is not kept");
        assert_eq!(cached(&mut last, || Some(false)), Network::Wifi);
        assert_eq!(cached(&mut last, || panic!("asked again within 5 s")), Network::Wifi);
    }
}
