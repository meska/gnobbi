# Vendored winit fork

Source: https://github.com/storytold/winit rev a517b3846ee8457d84ee4975110bfda6797c44dc (Apache-2.0). Unmodified sources, `docs/` dropped.

## Local patch: iOS UIScene (iOS 27 SDK)
Backport of molenick/winit@94d6a23 (rust-windowing/winit PR #4723, open, Apache-2.0): features `UIScene`/`UIWindowScene` in `Cargo.toml`, `os_capabilities().scenes` in `ios/app_state.rs`, and `ios/window.rs` attaches the new `UIWindow` to the connected `UIWindowScene`. The commit alone is not enough: the app Info.plist must declare `UIApplicationSceneManifest` with a `UISceneConfigurations`/`UIWindowSceneSessionRoleApplication` entry (no scene delegate), as in `packaging/ios/Info*.plist`; without it UIKit still aborts with "UIScene life cycle is required". Windows are created in `Resumed` (didBecomeActive), after the scene has connected, so the later PR head's deferral of `can_create_surfaces` was not needed. Drop it when a winit release contains #4723.
