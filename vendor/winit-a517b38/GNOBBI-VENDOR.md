# Vendored winit fork

Source: https://github.com/storytold/winit rev a517b3846ee8457d84ee4975110bfda6797c44dc (Apache-2.0). Unmodified sources, `docs/` dropped.

## Local patch: iOS UIScene (iOS 27 SDK)
Backport of molenick/winit@94d6a23 (rust-windowing/winit PR #4723, open, Apache-2.0): features `UIScene`/`UIWindowScene` in `Cargo.toml`, `os_capabilities().scenes` in `ios/app_state.rs`, and `ios/window.rs` attaches the new `UIWindow` to the connected `UIWindowScene`. The commit alone is not enough: the app Info.plist must declare `UIApplicationSceneManifest` with a `UISceneConfigurations`/`UIWindowSceneSessionRoleApplication` entry (no scene delegate), as in `packaging/ios/Info*.plist`; without it UIKit still aborts with "UIScene life cycle is required". Windows are created in `Resumed` (didBecomeActive), after the scene has connected, so the later PR head's deferral of `can_create_surfaces` was not needed. Drop it when a winit release contains #4723.

## Local patch: iOS UITextInput (CJK composition, autocorrection, candidate position)
Base: rust-windowing/winit PR #4592 (open, head e49d771a, Apache-2.0): `WinitView` adopts `UITextInput`, marked text becomes `Ime::Preedit`, a candidate or `unmarkText` becomes `Ime::Commit`. Changes on top of the PR (`ios/ime.rs` keeps its position/range classes, `ios/ime_document.rs` is new):
- The document UIKit sees is the text typed since the caret last moved plus the marked text (`ImeDocument`, UTF-16 offsets, the same design as macOS `ime_document.rs`), not only the marked text: that is what lets autocorrection (`replaceRange:withText:`, `textInRange:`) work. A replacement that reaches the caret becomes `Ime::Commit` with the replaced bytes available once from `WindowExtIOS::take_ime_delete_surrounding` (egui-winit forwards it as `DeleteSurroundingBytes`, like macOS/Wayland); ranges the document can't answer are ignored.
- The PR committed the old marked text on `setMarkedText:nil`: that turned Backspace on the last pinyin letter into a stray letter. Here nil/empty discards the composition; only `unmarkText` and a tap commit it. `insertText:` with a composition replaces it. Backspace in a composition edits only the composition.
- UTF-16 `NSRange` converted to UTF-8 offsets for `Preedit` (the PR used scalar counts).
- `Window::set_ime_cursor_area` positions `firstRectForRange:`/`caretRectForPosition:`; `set_ime_purpose(Password)` sets `isSecureTextEntry`, no autocorrection, and the document stays empty; `reset_ime`/`reset_ime_with_request` and `set_ime_allowed` drop the composition without committing (field change) and notify the `UITextInputDelegate`.
- Host test of the document model: `tests/ios_ime_document.rs`. egui-winit: `ime_delete_surrounding` has an iOS branch.
Drop it when a winit release contains #4592 and an equivalent of the autocorrection link.
