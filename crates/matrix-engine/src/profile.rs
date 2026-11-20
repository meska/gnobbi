//! Own-profile editing (display name, avatar) through the official SDK only; no UI, no own HTTP.
//! File paths never enter an error or a log line: errors are closed enums with fixed texts.
use image::GenericImageView;
use std::{io::Read, path::PathBuf, time::Duration};

/// Synapse refuses longer names (400), Tuwunel accepts them; we cap for both. Counted in characters.
pub const MAX_NAME_CHARS: usize = 256;
/// Side of the square picture uploaded (smaller pictures keep their size) and of the preview.
pub const AVATAR_SIDE: u32 = 512;
pub const PREVIEW_SIDE: u32 = 128;
const CALL_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    NameEmpty,
    NameTooLong,
    /// The server forbids changing this field (capability): nothing was attempted.
    NotAllowed,
    FileUnreadable,
    FileTooLarge,
    /// Not a PNG, JPEG or WebP picture, or undecodable.
    FileInvalid,
    Network,
    Rejected,
}

impl ProfileError {
    /// One line for the UI.
    pub fn message(self) -> &'static str {
        match self {
            Self::NameEmpty => "The name cannot be empty.",
            Self::NameTooLong => "The name is too long (256 characters at most).",
            Self::NotAllowed => "This server does not allow editing the profile.",
            Self::FileUnreadable => "Cannot read the chosen file.",
            Self::FileTooLarge => "The image is too large (25 MB at most).",
            Self::FileInvalid => "Choose a valid PNG, JPEG or WebP image.",
            Self::Network => "Connection failed: try again shortly.",
            Self::Rejected => "The server rejected the change.",
        }
    }
}

/// Trimmed name, or why it is not acceptable. Any Unicode (emoji included) is kept as typed.
pub fn validate_display_name(raw: &str) -> Result<String, ProfileError> {
    let name = raw.trim();
    if name.is_empty() {
        Err(ProfileError::NameEmpty)
    } else if name.chars().count() > MAX_NAME_CHARS {
        Err(ProfileError::NameTooLong)
    } else {
        Ok(name.to_owned())
    }
}

/// Square, bounded picture ready to upload (PNG) plus its RGBA preview.
#[derive(Clone)]
pub struct PreparedAvatar {
    pub png: Vec<u8>,
    pub preview: (u32, u32, Vec<u8>),
}

/// Reads at most `MAX_MEDIA_BYTES + 1` bytes of `path`, centre-crops to a square and bounds it.
pub async fn prepare_avatar(path: PathBuf) -> Result<PreparedAvatar, ProfileError> {
    tokio::task::spawn_blocking(move || prepare_blocking(&path))
        .await
        .map_err(|_| ProfileError::FileInvalid)?
}

/// The picture in a chosen file: at most `MAX_MEDIA_BYTES` are read, PNG, JPEG or WebP only, bounded in pixels.
pub fn decode_blocking(path: &std::path::Path) -> Result<image::DynamicImage, ProfileError> {
    let max = crate::media::MAX_MEDIA_BYTES;
    let mut data = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(max as u64 + 1).read_to_end(&mut data))
        .map_err(|_| ProfileError::FileUnreadable)?;
    decode_bytes(&data)
}

/// Same checks as `decode_blocking`, for bytes already in memory (the current avatar from the server).
pub fn decode_bytes(data: &[u8]) -> Result<image::DynamicImage, ProfileError> {
    if data.len() > crate::media::MAX_MEDIA_BYTES {
        return Err(ProfileError::FileTooLarge);
    }
    let format = match crate::media_sdk::mime_of(data) {
        Some("image/png") => image::ImageFormat::Png,
        Some("image/jpeg") => image::ImageFormat::Jpeg,
        Some("image/webp") => image::ImageFormat::WebP,
        _ => return Err(ProfileError::FileInvalid),
    };
    let reader = || {
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(data), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = None;
        limits.max_image_height = None;
        limits.max_alloc = Some(512 * 1024 * 1024);
        reader.limits(limits);
        reader
    };
    let (w, h) = reader().into_dimensions().map_err(|_| ProfileError::FileInvalid)?;
    if u64::from(w) * u64::from(h) > crate::media::MAX_IMAGE_PIXELS {
        return Err(ProfileError::FileTooLarge);
    }
    // Phone photos are stored sideways with an orientation tag: turn them the way they are meant to be seen.
    let mut decoder = reader().into_decoder().map_err(|_| ProfileError::FileInvalid)?;
    let orientation = image::ImageDecoder::orientation(&mut decoder).unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut picture = image::DynamicImage::from_decoder(decoder).map_err(|_| ProfileError::FileInvalid)?;
    picture.apply_orientation(orientation);
    Ok(picture)
}

/// Same as `prepare_avatar`, for callers that already run off the UI thread.
pub fn prepare_blocking(path: &std::path::Path) -> Result<PreparedAvatar, ProfileError> {
    let picture = decode_blocking(path)?;
    let (w, h) = picture.dimensions();
    let side = w.min(h).min(AVATAR_SIDE);
    let square = picture.resize_to_fill(side, side, image::imageops::FilterType::Triangle);
    let mut png = std::io::Cursor::new(Vec::new());
    square
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| ProfileError::FileInvalid)?;
    let preview_side = side.min(PREVIEW_SIDE);
    let preview = square.resize_exact(preview_side, preview_side, image::imageops::FilterType::Triangle);
    Ok(PreparedAvatar { png: png.into_inner(), preview: (preview_side, preview_side, preview.to_rgba8().into_raw()) })
}

/// What the server lets this account change; unknown (capabilities unreadable) counts as allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub name: bool,
    pub avatar: bool,
}

pub enum AvatarEdit {
    Keep,
    Remove,
    Set(PreparedAvatar),
}

/// The profile as shown, to save only what differs.
pub struct Current {
    pub name: Option<String>,
    pub has_avatar: bool,
}

/// What got written; also carried by a failure, so a retry does not redo it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
    pub name: bool,
    pub avatar: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SaveError {
    pub done: Saved,
    pub error: ProfileError,
}

/// The few SDK calls the flow needs, so the flow can be tested without a server.
#[allow(async_fn_in_trait)]
pub trait ProfileApi {
    async fn capabilities(&self) -> Capabilities;
    async fn set_name(&self, name: &str) -> Result<(), ProfileError>;
    async fn set_avatar(&self, png: Vec<u8>) -> Result<(), ProfileError>;
    async fn remove_avatar(&self) -> Result<(), ProfileError>;
}

/// Validates, checks capabilities (a forbidden field is never attempted) and writes only changes.
pub async fn save_profile(
    api: &impl ProfileApi,
    current: &Current,
    new_name: &str,
    avatar: AvatarEdit,
) -> Result<Saved, SaveError> {
    let fail = |done, error| SaveError { done, error };
    let name = validate_display_name(new_name).map_err(|e| fail(Saved::default(), e))?;
    let name_changed = current.name.as_deref().map(str::trim) != Some(name.as_str());
    let avatar_changed = match avatar {
        AvatarEdit::Keep => false,
        AvatarEdit::Remove => current.has_avatar,
        AvatarEdit::Set(_) => true,
    };
    let mut done = Saved::default();
    if !name_changed && !avatar_changed {
        return Ok(done);
    }
    let caps = api.capabilities().await;
    if (name_changed && !caps.name) || (avatar_changed && !caps.avatar) {
        return Err(fail(done, ProfileError::NotAllowed));
    }
    if name_changed {
        api.set_name(&name).await.map_err(|e| fail(done, e))?;
        done.name = true;
    }
    if avatar_changed {
        match avatar {
            AvatarEdit::Set(picture) => api.set_avatar(picture.png).await,
            _ => api.remove_avatar().await,
        }
        .map_err(|e| fail(done, e))?;
        done.avatar = true;
    }
    Ok(done)
}

fn classify(error: &matrix_sdk::Error) -> ProfileError {
    if error.as_client_api_error().is_some() { ProfileError::Rejected } else { ProfileError::Network }
}

async fn bounded<T>(call: impl std::future::Future<Output = matrix_sdk::Result<T>>) -> Result<T, ProfileError> {
    tokio::time::timeout(CALL_DEADLINE, call)
        .await
        .map_err(|_| ProfileError::Network)?
        .map_err(|e| classify(&e))
}

impl ProfileApi for matrix_sdk::Client {
    async fn capabilities(&self) -> Capabilities {
        let caps = self.homeserver_capabilities();
        // An unreadable answer is "don't know": the controls stay on and the server decides.
        Capabilities {
            name: caps.can_change_displayname().await.unwrap_or(true),
            avatar: caps.can_change_avatar().await.unwrap_or(true),
        }
    }
    async fn set_name(&self, name: &str) -> Result<(), ProfileError> {
        bounded(self.account().set_display_name(Some(name))).await
    }
    async fn set_avatar(&self, png: Vec<u8>) -> Result<(), ProfileError> {
        bounded(self.account().upload_avatar(&mime_guess::mime::IMAGE_PNG, png)).await.map(|_| ())
    }
    async fn remove_avatar(&self) -> Result<(), ProfileError> {
        bounded(self.account().set_avatar_url(None)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn name_validation() {
        assert_eq!(validate_display_name("  Ada  ").unwrap(), "Ada");
        assert_eq!(validate_display_name(""), Err(ProfileError::NameEmpty));
        assert_eq!(validate_display_name(" \t\n "), Err(ProfileError::NameEmpty));
        assert_eq!(validate_display_name("Zoë 日本 🦀👨‍👩‍👧").unwrap(), "Zoë 日本 🦀👨‍👩‍👧");
        assert!(validate_display_name(&"🦀".repeat(MAX_NAME_CHARS)).is_ok());
        assert_eq!(validate_display_name(&"a".repeat(MAX_NAME_CHARS + 1)), Err(ProfileError::NameTooLong));
        assert_eq!(validate_display_name(&format!(" {} ", "a".repeat(MAX_NAME_CHARS))).map(|n| n.len()), Ok(MAX_NAME_CHARS));
    }

    struct Fake {
        caps: Capabilities,
        fail: Option<ProfileError>,
        calls: RefCell<Vec<&'static str>>,
    }
    impl Fake {
        fn new(name: bool, avatar: bool, fail: Option<ProfileError>) -> Self {
            Self { caps: Capabilities { name, avatar }, fail, calls: RefCell::default() }
        }
        fn call(&self, what: &'static str) -> Result<(), ProfileError> {
            self.calls.borrow_mut().push(what);
            self.fail.map_or(Ok(()), Err)
        }
    }
    impl ProfileApi for Fake {
        async fn capabilities(&self) -> Capabilities { self.caps }
        async fn set_name(&self, _: &str) -> Result<(), ProfileError> { self.call("name") }
        async fn set_avatar(&self, _: Vec<u8>) -> Result<(), ProfileError> { self.call("set_avatar") }
        async fn remove_avatar(&self) -> Result<(), ProfileError> { self.call("remove_avatar") }
    }

    fn cur() -> Current { Current { name: Some("Ada".into()), has_avatar: true } }
    fn picture() -> PreparedAvatar { PreparedAvatar { png: vec![1], preview: (1, 1, vec![0; 4]) } }

    #[tokio::test]
    async fn saves_only_changes() {
        let api = Fake::new(true, true, None);
        let saved = save_profile(&api, &cur(), " Ada ", AvatarEdit::Keep).await.unwrap();
        assert_eq!((saved, api.calls.borrow().len()), (Saved::default(), 0));
        let saved = save_profile(&api, &cur(), "Bea", AvatarEdit::Set(picture())).await.unwrap();
        assert_eq!(saved, Saved { name: true, avatar: true });
        assert_eq!(*api.calls.borrow(), ["name", "set_avatar"]);
        api.calls.borrow_mut().clear();
        save_profile(&api, &cur(), "Ada", AvatarEdit::Remove).await.unwrap();
        save_profile(&api, &Current { name: None, has_avatar: false }, "Ada", AvatarEdit::Remove).await.unwrap();
        assert_eq!(*api.calls.borrow(), ["remove_avatar", "name"]);
    }

    #[tokio::test]
    async fn invalid_name_and_server_error() {
        let api = Fake::new(true, true, None);
        let err = save_profile(&api, &cur(), "  ", AvatarEdit::Keep).await.unwrap_err();
        assert_eq!(err.error, ProfileError::NameEmpty);
        assert!(api.calls.borrow().is_empty());
        let api = Fake::new(true, true, Some(ProfileError::Network));
        let err = save_profile(&api, &cur(), "Bea", AvatarEdit::Remove).await.unwrap_err();
        assert_eq!(err, SaveError { done: Saved::default(), error: ProfileError::Network });
    }

    #[tokio::test]
    async fn denied_capability_is_not_attempted() {
        let api = Fake::new(false, true, None);
        let err = save_profile(&api, &cur(), "Bea", AvatarEdit::Keep).await.unwrap_err();
        assert_eq!(err.error, ProfileError::NotAllowed);
        let api = Fake::new(true, false, None);
        let err = save_profile(&api, &cur(), "Ada", AvatarEdit::Set(picture())).await.unwrap_err();
        assert_eq!(err.error, ProfileError::NotAllowed);
        assert!(api.calls.borrow().is_empty());
        // Unchanged field stays fine even when its capability is off.
        save_profile(&api, &cur(), "Bea", AvatarEdit::Keep).await.unwrap();
    }

    fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("profile-test-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn encoded(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(w, h)).write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    #[tokio::test]
    async fn avatar_files() {
        let ok = temp("ok.png", &encoded(1200, 600, image::ImageFormat::Png));
        let p = prepare_avatar(ok.clone()).await.unwrap();
        let sq = image::load_from_memory(&p.png).unwrap();
        assert_eq!(sq.dimensions(), (AVATAR_SIDE.min(600), AVATAR_SIDE.min(600)));
        assert_eq!((p.preview.0, p.preview.1, p.preview.2.len()), (128, 128, 128 * 128 * 4));
        let jpg = temp("ok.jpg", &encoded(40, 80, image::ImageFormat::Jpeg));
        assert_eq!(image::load_from_memory(&prepare_avatar(jpg.clone()).await.unwrap().png).unwrap().dimensions(), (40, 40));

        let text = temp("bad.png", b"not a picture");
        let gif = temp("a.gif", &encoded(4, 4, image::ImageFormat::Gif));
        let huge = temp("huge.png", &vec![0; crate::media::MAX_MEDIA_BYTES + 1]);
        let missing = std::env::temp_dir().join("profile-test-missing-secret-dir/file.png");
        assert_eq!(prepare_avatar(text.clone()).await.err(), Some(ProfileError::FileInvalid));
        assert_eq!(prepare_avatar(gif.clone()).await.err(), Some(ProfileError::FileInvalid));
        assert_eq!(prepare_avatar(huge.clone()).await.err(), Some(ProfileError::FileTooLarge));
        let err = prepare_avatar(missing.clone()).await.err().unwrap();
        assert_eq!(err, ProfileError::FileUnreadable);
        // Messages are fixed texts: no path can appear.
        assert!(!format!("{err:?} {}", err.message()).contains("secret-dir"));
        for p in [ok, jpg, text, gif, huge] {
            let _ = std::fs::remove_file(p);
        }
    }
}
