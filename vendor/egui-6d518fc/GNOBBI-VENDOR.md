# Vendored egui fork

Source: https://github.com/storytold/egui rev 6d518fc799681571963842f95b55a6b68431b15a (egui 0.37.0, MIT OR Apache-2.0).
Unmodified sources. Kept: ecolor, emath, epaint, epaint_default_fonts, egui, egui-winit, egui-wgpu, egui_glow, eframe, egui_system_font_provider.
Dropped: demos, plots, web, kittest, inspection, extras, examples, tests. The root Cargo.toml is the upstream one with `members` trimmed to the kept crates (workspace inheritance).

## Local patch: iOS keyboard stays up on a tap and on a focus move (egui-winit)
`egui-winit/src/lib.rs`, iOS only (`cfg(target_os = "ios")`): winit's iOS `set_ime_allowed(false)` is `resignFirstResponder`, so every `set_ime_allowed(false)`+`(true)` made the keyboard slide down and up (measured in the simulator: 1 hide+show per tap in the focused field, also when switching fields, 2 per tap on the same field).
- The stock "interrupt composition" (`should_interrupt_composition`: a press in the focused field) no longer toggles `set_ime_allowed`; it calls `window.reset_ime()`, which drops the composition without committing and leaves the first responder alone.
- `ime_hold`: when egui stops asking for the IME (the old field loses focus a frame before the new one gains it), the request is held for `IOS_IME_HOLD` (150 ms, `request_repaint_after` guarantees a frame); only a request that stays off that long becomes `set_ime_allowed(false)`. Pure function with a unit test (`ios_ime_is_held_across_a_focus_move_but_released_for_real`).
- Cost: after a real focus loss the keyboard goes away 150 ms later. A purpose change between fields (Password/Terminal/Normal) still does `reloadInputViews` in winit (UIKit re-reads the traits): UIKit posts WillChangeFrame/WillShow with the same frame, no hide.
Drop it when egui-winit/winit handle this upstream.

## Local patch: IME purposes for sign-in AutoFill
- `IMEPurpose::{Username, Url, LoginPassword}` and `IMEPurpose::is_secret()` (Password and LoginPassword are secrets: used in `ime_context.rs`, `ime_surrounding.rs`, web `text_agent.rs`). `TextEdit::ime_purpose()` sets the purpose without changing how the text is drawn (the eye toggle of the sign-in password keeps `LoginPassword`).
- egui-winit: off iOS the new purposes map to Terminal/Password; on iOS to the winit ones. On iOS the purpose is sent BEFORE `set_ime_allowed(true)` so the traits are in place when the keyboard comes up, and `take_egui_input` moves `winit::platform::ios::take_autofill_credentials()` into egui's data store (`Id::new("winit_autofill_credentials")`, `(Option<String>, Option<String>)`).
