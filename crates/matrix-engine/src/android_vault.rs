//! Android Keystore protection-secret vault. One non-exportable AES-256-GCM Keystore key seals one
//! 32-byte secret per account store; the sealed blob lives in `noBackupFilesDir/gnobbi-vault`.
//! No Kotlin helper: Rust calls the framework classes over JNI. Never falls back to a plain file.
//! Blocking: call from a backend worker.
#[cfg(target_os = "android")]
use crate::{persistence::AccountStore, signin::ProtectionSource};
use crate::{
    fsx,
    signin::{ios_profile_item, secret_from_item, secret_from_random, Secret, VaultError},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
};
use zeroize::Zeroizing;

const VERSION: u8 = 1;
const IV_LEN: usize = 12;
/// version + iv + 64-byte hex secret + 16-byte GCM tag.
const FILE_LEN: usize = 1 + IV_LEN + 64 + 16;

/// Whole module under one lock: two concurrent first uses would generate the key twice and the
/// second key would orphan every blob sealed with the first.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
static LOCK: Mutex<()> = Mutex::new(());
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sealing with the Keystore key. A sealed value is `iv(12) || ciphertext || tag`.
trait Sealer {
    /// Creates the key when missing.
    fn seal(&self, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, VaultError>;
    /// A missing key is `InvalidItem`: it is never regenerated here.
    fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError>;
}

/// Maps a Java exception class name; everything unknown is `Unavailable`.
fn classify(name: &str) -> VaultError {
    match name {
        "javax.crypto.AEADBadTagException"
        | "javax.crypto.BadPaddingException"
        | "android.security.keystore.KeyPermanentlyInvalidatedException" => VaultError::InvalidItem,
        _ => VaultError::Unavailable,
    }
}

fn item_of(path: &Path) -> Result<String, VaultError> {
    ios_profile_item(path).ok_or(VaultError::Unavailable)
}

fn file_of(dir: &Path, item: &str) -> PathBuf {
    dir.join(format!("{}.gcm", item.replace('/', "_")))
}

fn available_in(dir: &Path, path: &Path) -> Result<bool, VaultError> {
    Ok(!file_of(dir, &item_of(path)?).exists())
}

fn create_in(dir: &Path, sealer: &dyn Sealer, path: &Path, random: &[u8; 32]) -> Result<Secret, VaultError> {
    let item = item_of(path)?;
    let dest = file_of(dir, &item);
    if dest.exists() {
        return Err(VaultError::AlreadyExists);
    }
    let secret = secret_from_random(random)?;
    let sealed = sealer.seal(item.as_bytes(), secret.0.as_bytes())?;
    if sealed.len() != FILE_LEN - 1 {
        return Err(VaultError::Unavailable);
    }
    let pending = dest.with_extension("gcm.pending");
    let io = |_: std::io::Error| VaultError::Unavailable;
    let _ = fsx::remove_pending(&pending);
    let mut file = fsx::private_file(fs::OpenOptions::new().write(true).create_new(true)).open(&pending).map_err(io)?;
    let written = file.write_all(&[VERSION]).and_then(|()| file.write_all(&sealed)).and_then(|()| file.sync_all());
    drop(file);
    let published = written.and_then(|()| fsx::publish_new(&pending, &dest));
    let _ = fsx::remove_pending(&pending);
    match published {
        Ok(()) => fsx::sync_dir(dir).map_err(io)?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(VaultError::AlreadyExists),
        Err(e) => return Err(io(e)),
    }
    Ok(secret)
}

fn read_in(dir: &Path, sealer: &dyn Sealer, path: &Path) -> Result<Secret, VaultError> {
    let item = item_of(path)?;
    let blob = match fs::read(file_of(dir, &item)) {
        Ok(blob) => blob,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(VaultError::Missing),
        Err(_) => return Err(VaultError::Unavailable),
    };
    if blob.len() != FILE_LEN || blob[0] != VERSION {
        return Err(VaultError::InvalidItem);
    }
    secret_from_item(&sealer.open(item.as_bytes(), &blob[1..])?)
}

fn delete_in(dir: &Path, path: &Path) -> Result<(), VaultError> {
    match fs::remove_file(file_of(dir, &item_of(path)?)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(VaultError::Unavailable),
    }
}

/// The Android Keystore as a profile protection source.
#[cfg(target_os = "android")]
pub struct AndroidKeystore;
#[cfg(target_os = "android")]
impl ProtectionSource for AndroidKeystore {
    fn available(&self, path: &Path) -> Result<bool, VaultError> {
        let _guard = lock();
        available_in(&bridge::vault_dir()?, path)
    }
    fn create(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        let _guard = lock();
        let mut random = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *random).map_err(|_| VaultError::Unavailable)?;
        create_in(&bridge::vault_dir()?, &bridge::Keystore, &store.path, &random)
    }
    fn read(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        let _guard = lock();
        read_in(&bridge::vault_dir()?, &bridge::Keystore, &store.path)
    }
}

/// Secret of one store (the passphrase of its sqlite store), for the push reader that runs without the engine.
#[cfg(target_os = "android")]
pub(crate) fn read_secret(path: &Path) -> Result<Secret, VaultError> {
    let _guard = lock();
    read_in(&bridge::vault_dir()?, &bridge::Keystore, path)
}

/// JavaVM and a never-released global ref to the application Context, from the FCM service when the process was started for a push
/// only (no android-activity, so `ndk_context` is empty). Never initializes `ndk_context`: android-activity does that itself later
/// and a second init panics (risk R1). `ndk_context`, when set, wins.
#[cfg(target_os = "android")]
pub fn init_context(vm: *mut std::ffi::c_void, context: *mut std::ffi::c_void) {
    let _ = bridge::SERVICE_CONTEXT.set((vm as usize, context as usize));
}

/// True once [`init_context`] ran (the caller then does not leak another global ref).
#[cfg(target_os = "android")]
pub fn context_ready() -> bool {
    bridge::SERVICE_CONTEXT.get().is_some()
}

/// Removes the sealed blob of one store; an absent blob is fine. The Keystore key stays for the other profiles.
#[cfg(target_os = "android")]
pub(crate) fn delete_profile_store(path: &Path) -> Result<(), VaultError> {
    let _guard = lock();
    delete_in(&bridge::vault_dir()?, path)
}

#[cfg(target_os = "android")]
mod bridge {
    use super::{classify, fsx, Sealer, VaultError, IV_LEN};
    use jni::{
        errors::Error,
        jni_sig, jni_str,
        objects::{JByteArray, JObject, JObjectArray, JString},
        Env, JValue,
    };
    use std::path::PathBuf;
    use zeroize::Zeroizing;

    const ALIAS: &str = "gnobbi.sdk-protection-v1";

    /// Jni errors become `Unavailable` unless a Java exception names a known cause. Only the exception
    /// class is logged: the `Display` of a jni error carries the Java message and stack.
    fn map(error: Error) -> VaultError {
        match error {
            Error::CaughtJavaException { name, .. } => {
                let mapped = classify(&name);
                tracing::warn!("android vault exc={name} result={mapped:?}");
                mapped
            }
            _ => {
                tracing::warn!("android vault jni error");
                VaultError::Unavailable
            }
        }
    }

    pub(super) static SERVICE_CONTEXT: std::sync::OnceLock<(usize, usize)> = std::sync::OnceLock::new();

    /// (JavaVM, Context): android-activity's through ndk-context (panics when it never ran), else the service's.
    fn raw_context() -> Result<(*mut std::ffi::c_void, *mut std::ffi::c_void), VaultError> {
        if let Ok(ctx) = std::panic::catch_unwind(ndk_context::android_context) {
            return Ok((ctx.vm(), ctx.context()));
        }
        SERVICE_CONTEXT.get().map(|&(vm, ctx)| (vm as *mut _, ctx as *mut _)).ok_or(VaultError::Unavailable)
    }

    fn attach<T>(f: impl FnOnce(&mut Env, *mut std::ffi::c_void) -> Result<T, Error>) -> Result<T, VaultError> {
        let (vm, context) = raw_context()?;
        // SAFETY: the VM pointer comes from android-activity or the service; from_raw is idempotent.
        let vm = unsafe { jni::JavaVM::from_raw(vm.cast()) };
        vm.attach_current_thread(|env| f(env, context)).map_err(map)
    }

    pub(super) fn vault_dir() -> Result<PathBuf, VaultError> {
        let dir = attach(|env, context| {
            // SAFETY: a global ref that is never released (android-activity's or the service's); JObject has no Drop and is not kept.
            let context = unsafe { JObject::from_raw(env, context.cast()) };
            let file = env.call_method(&context, jni_str!("getNoBackupFilesDir"), jni_sig!("()Ljava/io/File;"), &[])?.l()?;
            let path = env.call_method(&file, jni_str!("getAbsolutePath"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
            let path = env.cast_local::<JString>(path)?;
            Ok(path.try_to_string(env)?)
        })?;
        let dir = PathBuf::from(dir).join("gnobbi-vault");
        match fsx::create_private_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(VaultError::Unavailable),
        }
        Ok(dir)
    }

    fn jstr<'l>(env: &mut Env<'l>, s: &str) -> Result<JString<'l>, Error> {
        env.new_string(s)
    }

    /// The Keystore key, or `None` when the alias is absent.
    fn key<'l>(env: &mut Env<'l>) -> Result<Option<JObject<'l>>, Error> {
        let kind = jstr(env, "AndroidKeyStore")?;
        let ks = env
            .call_static_method(
                jni_str!("java/security/KeyStore"),
                jni_str!("getInstance"),
                jni_sig!("(Ljava/lang/String;)Ljava/security/KeyStore;"),
                &[JValue::from(&kind)],
            )?
            .l()?;
        env.call_method(&ks, jni_str!("load"), jni_sig!("(Ljava/security/KeyStore$LoadStoreParameter;)V"), &[JValue::from(&JObject::null())])?;
        let alias = jstr(env, ALIAS)?;
        let key = env
            .call_method(
                &ks,
                jni_str!("getKey"),
                jni_sig!("(Ljava/lang/String;[C)Ljava/security/Key;"),
                &[JValue::from(&alias), JValue::from(&JObject::null())],
            )?
            .l()?;
        Ok(if key.is_null() { None } else { Some(key) })
    }

    fn one<'l>(env: &mut Env<'l>, s: &str) -> Result<JObjectArray<'l, JString<'l>>, Error> {
        let s = jstr(env, s)?;
        JObjectArray::<JString>::new(env, 1, &s)
    }

    fn generate_key(env: &mut Env) -> Result<(), Error> {
        let alias = jstr(env, ALIAS)?;
        let builder = env.new_object(
            jni_str!("android/security/keystore/KeyGenParameterSpec$Builder"),
            jni_sig!("(Ljava/lang/String;I)V"),
            &[JValue::from(&alias), JValue::Int(3)], // PURPOSE_ENCRYPT | PURPOSE_DECRYPT
        )?;
        let builder_sig = jni_sig!("([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;");
        let modes = one(env, "GCM")?;
        env.call_method(&builder, jni_str!("setBlockModes"), &builder_sig, &[JValue::from(&modes)])?;
        let paddings = one(env, "NoPadding")?;
        env.call_method(&builder, jni_str!("setEncryptionPaddings"), &builder_sig, &[JValue::from(&paddings)])?;
        env.call_method(&builder, jni_str!("setKeySize"), jni_sig!("(I)Landroid/security/keystore/KeyGenParameterSpec$Builder;"), &[JValue::Int(256)])?;
        let spec = env.call_method(&builder, jni_str!("build"), jni_sig!("()Landroid/security/keystore/KeyGenParameterSpec;"), &[])?.l()?;
        let (algorithm, provider) = (jstr(env, "AES")?, jstr(env, "AndroidKeyStore")?);
        let generator = env
            .call_static_method(
                jni_str!("javax/crypto/KeyGenerator"),
                jni_str!("getInstance"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;"),
                &[JValue::from(&algorithm), JValue::from(&provider)],
            )?
            .l()?;
        env.call_method(&generator, jni_str!("init"), jni_sig!("(Ljava/security/spec/AlgorithmParameterSpec;)V"), &[JValue::from(&spec)])?;
        env.call_method(&generator, jni_str!("generateKey"), jni_sig!("()Ljavax/crypto/SecretKey;"), &[])?;
        Ok(())
    }

    fn cipher<'l>(env: &mut Env<'l>) -> Result<JObject<'l>, Error> {
        let name = jstr(env, "AES/GCM/NoPadding")?;
        env.call_static_method(
            jni_str!("javax/crypto/Cipher"),
            jni_str!("getInstance"),
            jni_sig!("(Ljava/lang/String;)Ljavax/crypto/Cipher;"),
            &[JValue::from(&name)],
        )?
        .l()
    }

    /// `updateAAD` + `doFinal`; the Java copy of the input is zeroed afterwards, the output is copied out.
    fn finish(env: &mut Env, cipher: &JObject, aad: &[u8], input: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        let aad = env.byte_array_from_slice(aad)?;
        env.call_method(cipher, jni_str!("updateAAD"), jni_sig!("([B)V"), &[JValue::from(&aad)])?;
        let input_array = env.byte_array_from_slice(input)?;
        let out = env.call_method(cipher, jni_str!("doFinal"), jni_sig!("([B)[B"), &[JValue::from(&input_array)]);
        input_array.set_region(env, 0, &vec![0i8; input.len()])?;
        let out = env.cast_local::<JByteArray>(out?.l()?)?;
        let bytes = Zeroizing::new(env.convert_byte_array(&out)?);
        out.set_region(env, 0, &vec![0i8; bytes.len()])?;
        Ok(bytes)
    }

    pub(super) struct Keystore;
    impl Sealer for Keystore {
        fn seal(&self, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, VaultError> {
            attach(|env, _| {
                let key = match key(env)? {
                    Some(key) => key,
                    None => {
                        generate_key(env)?;
                        key(env)?.ok_or(Error::NullPtr("keystore key"))?
                    }
                };
                let cipher = cipher(env)?;
                env.call_method(&cipher, jni_str!("init"), jni_sig!("(ILjava/security/Key;)V"), &[JValue::Int(1), JValue::from(&key)])?;
                let iv = env.call_method(&cipher, jni_str!("getIV"), jni_sig!("()[B"), &[])?.l()?;
                let iv = env.convert_byte_array(&env.cast_local::<JByteArray>(iv)?)?;
                let sealed = finish(env, &cipher, aad, plain)?;
                Ok([iv, sealed.to_vec()].concat())
            })
        }

        fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
            if sealed.len() <= IV_LEN {
                return Err(VaultError::InvalidItem);
            }
            let (iv, data) = sealed.split_at(IV_LEN);
            let opened = attach(|env, _| {
                let Some(key) = key(env)? else { return Ok(None) };
                let cipher = cipher(env)?;
                let iv = env.byte_array_from_slice(iv)?;
                let spec = env.new_object(jni_str!("javax/crypto/spec/GCMParameterSpec"), jni_sig!("(I[B)V"), &[JValue::Int(128), JValue::from(&iv)])?;
                env.call_method(
                    &cipher,
                    jni_str!("init"),
                    jni_sig!("(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V"),
                    &[JValue::Int(2), JValue::from(&key), JValue::from(&spec)],
                )?;
                finish(env, &cipher, aad, data).map(Some)
            })?;
            opened.ok_or(VaultError::InvalidItem)
        }
    }
}

#[cfg(test)]
#[path = "android_vault_tests.rs"]
mod tests;
