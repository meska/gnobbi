use matrix_desktop::{AppDirs, Platform, app_dirs};
use std::path::Path;

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |key| {
        pairs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_owned())
    }
}

#[test]
fn linux_defaults_to_home_relative_xdg_directories() {
    let d = app_dirs(Platform::Linux, env(&[("HOME", "/home/u")])).unwrap();
    assert_eq!(d.config, Path::new("/home/u/.config/gnobbi"));
    assert_eq!(d.data, Path::new("/home/u/.local/share/gnobbi"));
    assert_eq!(d.cache, Path::new("/home/u/.cache/gnobbi"));
}

#[test]
fn linux_honours_absolute_xdg_variables() {
    let vars = [
        ("HOME", "/home/u"),
        ("XDG_CONFIG_HOME", "/c"),
        ("XDG_DATA_HOME", "/d"),
        ("XDG_CACHE_HOME", "/k"),
    ];
    let d = app_dirs(Platform::Linux, env(&vars)).unwrap();
    assert_eq!(d.config, Path::new("/c/gnobbi"));
    assert_eq!(d.data, Path::new("/d/gnobbi"));
    assert_eq!(d.cache, Path::new("/k/gnobbi"));
}

#[test]
fn linux_ignores_empty_and_relative_xdg_variables() {
    let vars = [
        ("HOME", "/home/u"),
        ("XDG_CONFIG_HOME", ""),
        ("XDG_DATA_HOME", "rel/data"),
    ];
    let d = app_dirs(Platform::Linux, env(&vars)).unwrap();
    assert_eq!(d.config, Path::new("/home/u/.config/gnobbi"));
    assert_eq!(d.data, Path::new("/home/u/.local/share/gnobbi"));
}

#[test]
fn linux_without_a_usable_home_has_no_directories() {
    assert!(app_dirs(Platform::Linux, env(&[])).is_none());
    assert!(app_dirs(Platform::Linux, env(&[("HOME", "relative")])).is_none());
}

#[test]
fn macos_uses_application_support_and_caches_under_the_sandbox_home() {
    let home = "/Users/x/Library/Containers/net.meskatech.dime/Data";
    let d = app_dirs(Platform::MacOs, env(&[("HOME", home)])).unwrap();
    let app_support = Path::new(home).join("Library/Application Support/Gnobbi");
    assert_eq!(d.config, app_support);
    assert_eq!(d.data, app_support);
    assert_eq!(d.cache, Path::new(home).join("Library/Caches/Gnobbi"));
}

#[test]
fn windows_uses_appdata_and_localappdata() {
    let vars = [("APPDATA", "C:\\R"), ("LOCALAPPDATA", "C:\\L")];
    let d = app_dirs(Platform::Windows, env(&vars)).unwrap();
    assert_eq!(d.config, Path::new("C:\\R").join("Gnobbi"));
    assert_eq!(d.data, Path::new("C:\\L").join("Gnobbi"));
    assert_eq!(d.cache, Path::new("C:\\L").join("Gnobbi").join("cache"));
    assert!(app_dirs(Platform::Windows, env(&[])).is_none());
}

#[cfg(unix)]
#[test]
fn ensure_creates_every_directory_private() {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!("dime-appdirs-{}", std::process::id()));
    let d = AppDirs {
        config: base.join("c/gnobbi"),
        data: base.join("d/gnobbi"),
        cache: base.join("k/gnobbi"),
    };
    d.ensure().unwrap();
    d.ensure().unwrap(); // idempotent
    for p in [&d.config, &d.data, &d.cache] {
        assert_eq!(
            std::fs::metadata(p).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    std::fs::remove_dir_all(&base).unwrap();
}
