//! «Informazioni su Gnobbi»: a page inside the side drawer (like Settings), opened from the version
//! line. Everything shown comes from `BUILD_INFO`; no account data, no network.
use crate::*;
use gpui::img;
use matrix_desktop::build_info::{
    BUILD_INFO, LICENSE_APACHE, LICENSE_MIT, THIRD_PARTY_NOTICES, system_version,
};

/// Back, Licenza, Licenze di terze parti, Copia informazioni, Chiudi.
const ABOUT_ROWS: usize = 5;

impl Shell {
    pub(crate) fn set_about_open(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.about_open = open;
        self.about_row = 0;
        if open {
            self.about_system = system_version();
            if self.about_icon.is_none() {
                self.about_icon = app_icon();
            }
            self.about_focus.focus(window)
        } else {
            self.drawer_focus.focus(window)
        }
        cx.notify();
    }

    /// Writes `text` under the cache folder and opens it with the system's default app.
    fn open_text(&self, name: &str, text: &str, cx: &mut Context<Self>) {
        let dir = self
            .prefs_dirs
            .as_ref()
            .map_or_else(std::env::temp_dir, |d| d.cache.clone());
        let path = dir.join(name);
        let written =
            matrix_desktop::create_private_dir(&dir).and_then(|()| std::fs::write(&path, text));
        match written {
            Ok(()) => {
                if let Ok(url) = url::Url::from_file_path(&path) {
                    cx.open_url(url.as_str());
                }
            }
            Err(error) => eprintln!("Gnobbi: cannot write {name}: {error}"),
        }
    }

    fn activate_about(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        match row {
            0 => self.set_about_open(false, window, cx),
            1 => self.open_text(
                "LICENSE.txt",
                &format!("{LICENSE_MIT}\n\n----\n\n{LICENSE_APACHE}"),
                cx,
            ),
            2 => self.open_text("THIRD_PARTY_NOTICES.md", THIRD_PARTY_NOTICES, cx),
            3 => cx.write_to_clipboard(ClipboardItem::new_string(
                BUILD_INFO.copy_text(&self.about_system),
            )),
            _ => self.set_menu_open(false, window, cx),
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_about(
        &self,
    ) -> Option<matrix_desktop::accessibility::about::AboutSnapshot> {
        (self.menu_open && self.about_open).then(|| {
            matrix_desktop::accessibility::about::AboutSnapshot {
                focus: self.about_row,
                system: self.about_system.clone(),
            }
        })
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_about(
        &mut self,
        action: matrix_desktop::accessibility::about::AboutAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::about::AboutAction;
        if !(self.menu_open && self.about_open) {
            return;
        }
        match action {
            AboutAction::Focus(row) => self.about_row = row,
            AboutAction::Activate(row) => {
                self.about_row = row;
                self.activate_about(row, window, cx)
            }
        }
        cx.notify();
    }

    /// Keys while the About page is shown (the drawer is open); true when consumed.
    pub(crate) fn about_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        let m = &key.modifiers;
        match key.key.as_str() {
            "escape" => self.set_about_open(false, window, cx),
            "enter" | "space" if !m.modified() => {
                self.activate_about(self.about_row, window, cx);
                cx.notify();
            }
            "up" | "down" | "tab" => {
                let back = key.key == "up" || m.shift;
                self.about_row =
                    (self.about_row + if back { ABOUT_ROWS - 1 } else { 1 }) % ABOUT_ROWS;
                cx.notify();
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn render_about(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let focused = self.about_focus.is_focused(window);
        let action = |index: usize, id: &'static str, label: &'static str| {
            div()
                .id(id)
                .h(px(44.))
                .mx_2()
                .px_3()
                .flex()
                .items_center()
                .rounded_lg()
                .cursor_pointer()
                .border_1()
                .border_color(rgb(if input_modality::only_keys(focused, cx) && self.about_row == index {
                    t.accent
                } else {
                    t.surface
                }))
                .text_color(rgb(t.text))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.about_row = index;
                    this.activate_about(index, window, cx)
                }))
        };
        let info = |label: &'static str, value: String| {
            div()
                .id(label)
                .min_h(px(32.))
                .py_1()
                .mx_2()
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(div().flex_shrink_0().text_color(rgb(t.secondary)).child(label))
                .child(div().flex_1().min_w_0().text_right().text_color(rgb(t.text)).child(value))
        };
        div()
            .id("about-page")
            .track_focus(&self.about_focus)
            .child(
                div().flex().items_center().justify_between()
                    .child(action(0, "about-back", "‹ Indietro"))
                    .child(
                        div().id("about-close").w(px(32.)).h(px(32.)).flex().items_center().justify_center().cursor_pointer()
                            .border_1()
                            .border_color(rgb(if input_modality::only_keys(focused, cx) && self.about_row == 4 { t.accent } else { t.surface }))
                            .hover(|s| s.bg(rgb(t.border)))
                            .child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.accent))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.about_row = 4;
                                this.activate_about(4, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .children(self.about_icon.clone().map(|icon| {
                        img(icon)
                            .size(px(64.))
                            .flex_shrink_0()
                            .object_fit(ObjectFit::Contain)
                    }))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(t.text))
                            .child("Gnobbi"),
                    ),
            )
            .child(info("Versione", BUILD_INFO.version.into()))
            .child(info("Build", BUILD_INFO.build.into()))
            .child(info("Commit", BUILD_INFO.git_sha.into()))
            .child(info("Data build", BUILD_INFO.build_date.into()))
            .child(info("Matrix SDK", BUILD_INFO.matrix_sdk.into()))
            .child(info("Sistema", self.about_system.clone()))
            .child(info("Sito", "gnobbi.com · gnobbi.chat".into()))
            .child(info("Licenza", "MIT OR Apache-2.0".into()))
            .child(action(1, "about-license", "Apri licenze"))
            .child(action(2, "about-third-party", "Licenze di terze parti"))
            .child(action(3, "about-copy", "Copia informazioni"))
            .into_any_element()
    }
}

/// The packaged app icon, decoded once for the About page.
fn app_icon() -> Option<std::sync::Arc<RenderImage>> {
    let png = include_bytes!("../../../packaging/linux/icon-128.png");
    let mut rgba = image::load_from_memory(png).ok()?.into_rgba8();
    // GPUI RenderImage expects BGRA.
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(std::sync::Arc::new(RenderImage::new([image::Frame::new(
        rgba,
    )])))
}
