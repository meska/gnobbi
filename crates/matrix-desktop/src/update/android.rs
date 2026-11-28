//! Android bridge of the updater to `net.meskatech.gnobbi.Native` (`selfUpdate`, `installApk`). The classes of the app are not
//! visible to `FindClass` on a native thread, so the class comes from the context's class loader.

use std::path::Path;

// Codes returned by `Native.installApk` (SelfUpdate.java).
pub(super) const OPENED: i32 = 0;
pub(super) const SIGNATURE: i32 = 1;
pub(super) const PERMISSION: i32 = 2;
pub(super) const UNAVAILABLE: i32 = 3;

/// Signature check and system installer for a staged APK; anything that goes wrong in JNI is `UNAVAILABLE`.
pub(super) fn open_installer(path: &Path) -> i32 {
    #[cfg(target_os = "android")]
    if let Some(code) = bridge::install(&path.to_string_lossy()) {
        return code;
    }
    #[cfg(not(target_os = "android"))]
    let _ = path;
    UNAVAILABLE
}

/// `BuildConfig.SELF_UPDATE`; `None` when the call fails.
#[cfg(target_os = "android")]
pub(super) fn self_update() -> Option<bool> {
    bridge::self_update()
}

#[cfg(target_os = "android")]
mod bridge {
    use jni::{errors::Error, jni_sig, jni_str, objects::{JClass, JObject, JValue}, Env};

    fn native_class<'l>(env: &mut Env<'l>, context: &JObject) -> Result<JClass<'l>, Error> {
        let loader = env.call_method(context, jni_str!("getClassLoader"), jni_sig!("()Ljava/lang/ClassLoader;"), &[])?.l()?;
        let name = env.new_string("net.meskatech.gnobbi.Native")?;
        let class = env.call_method(&loader, jni_str!("loadClass"), jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"), &[JValue::from(&name)])?.l()?;
        env.cast_local::<JClass>(class)
    }

    pub fn self_update() -> Option<bool> {
        let ctx = std::panic::catch_unwind(ndk_context::android_context).ok()?;
        // SAFETY: the VM pointer comes from android-activity; from_raw is idempotent.
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
        let res = vm.attach_current_thread(|env| -> Result<bool, Error> {
            // SAFETY: the context is a global ref owned by android-activity; JObject has no Drop and is not kept.
            let context = unsafe { JObject::from_raw(env, ctx.context().cast()) };
            let class = native_class(env, &context)?;
            env.call_static_method(&class, jni_str!("selfUpdate"), jni_sig!("()Z"), &[])?.z()
        });
        res.map_err(|e| log::warn!("gnobbi update: {e:?}")).ok()
    }

    pub fn install(path: &str) -> Option<i32> {
        let ctx = std::panic::catch_unwind(ndk_context::android_context).ok()?;
        // SAFETY: as in self_update.
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
        let res = vm.attach_current_thread(|env| -> Result<i32, Error> {
            // SAFETY: as in self_update.
            let context = unsafe { JObject::from_raw(env, ctx.context().cast()) };
            let class = native_class(env, &context)?;
            let path = env.new_string(path)?;
            env.call_static_method(&class, jni_str!("installApk"), jni_sig!("(Landroid/content/Context;Ljava/lang/String;)I"), &[JValue::from(&context), JValue::from(&path)])?.i()
        });
        res.map_err(|e| log::warn!("gnobbi update: {e:?}")).ok()
    }
}
