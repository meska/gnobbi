//! Display only bounded, already-decoded backend pixels. Never an image URL.
use crate::Shell;
use gpui::{div, img, prelude::*, px, rgb};
impl Shell {
    pub(crate) fn update_media_image(&mut self) {
        let Some(mut preview) = self.media.take_preview() else {
            return;
        };
        // GPUI RenderImage expects BGRA, image decoder returns straight RGBA.
        for pixel in preview.rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        if let Some(buffer) =
            image::RgbaImage::from_raw(preview.width, preview.height, preview.rgba)
        {
            self.media_image = Some(std::sync::Arc::new(gpui::RenderImage::new([
                image::Frame::new(buffer),
            ])));
        }
    }

    pub(crate) fn render_media(&self) -> gpui::AnyElement {
        let t = if self.dark {
            matrix_desktop::DARK
        } else {
            matrix_desktop::LIGHT
        };
        if let Some(image) = &self.media_image {
            let dimensions = image.size(0);
            return div()
                .flex_shrink_0()
                .px_4()
                .py_2()
                .bg(rgb(t.surface))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(t.secondary))
                        .child("Anteprima PNG · solo memoria"),
                )
                .child(
                    img(image.clone())
                        .w(px(dimensions.width.0 as f32))
                        .h(px(dimensions.height.0 as f32)),
                )
                .into_any_element();
        }
        if self.media.loading() || self.media.error().is_some() {
            return div()
                .flex_shrink_0()
                .px_4()
                .py_2()
                .bg(rgb(t.surface))
                .text_size(px(12.))
                .text_color(rgb(if self.media.loading() {
                    t.secondary
                } else {
                    t.error
                }))
                .child(if self.media.loading() {
                    "Caricamento PNG…"
                } else {
                    "Anteprima non disponibile · solo PNG valido ≤4 MiB / 2048 px"
                })
                .into_any_element();
        }
        div().into_any_element()
    }
}
