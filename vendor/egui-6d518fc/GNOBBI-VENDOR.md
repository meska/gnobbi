# Vendored egui fork

Source: https://github.com/storytold/egui rev 6d518fc799681571963842f95b55a6b68431b15a (egui 0.37.0, MIT OR Apache-2.0).
Unmodified sources. Kept: ecolor, emath, epaint, epaint_default_fonts, egui, egui-winit, egui-wgpu, egui_glow, eframe, egui_system_font_provider.
Dropped: demos, plots, web, kittest, inspection, extras, examples, tests. The root Cargo.toml is the upstream one with `members` trimmed to the kept crates (workspace inheritance).
