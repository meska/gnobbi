//! Pictures drawn inside message bubbles from bounded, already-decoded backend pixels. Never an image URL.
use crate::Shell;
use gpui::{AnyElement, div, img, prelude::*, px, rgb};
use matrix_desktop::DesignTokens;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) enum InlineImage {
    Loading,
    Ready(Arc<gpui::RenderImage>),
    Failed,
}

pub(crate) fn inline_picture(state: Option<InlineImage>, t: DesignTokens) -> AnyElement {
    match state {
        Some(InlineImage::Ready(image)) => {
            let size = image.size(0);
            img(image)
                .w(px(size.width.0 as f32))
                .h(px(size.height.0 as f32))
                .rounded_md()
                .into_any_element()
        }
        Some(InlineImage::Failed) => div()
            .w(px(200.))
            .h(px(60.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .bg(rgb(t.timeline))
            .text_size(px(12.))
            .text_color(rgb(t.error))
            .child("Immagine non disponibile")
            .into_any_element(),
        _ => div()
            .w(px(200.))
            .h(px(150.))
            .rounded_md()
            .bg(rgb(t.timeline))
            .into_any_element(),
    }
}

impl Shell {
    /// Ask for the next image of the open room, one read at a time; true when the view changed.
    pub(crate) fn drive_inline_images(&mut self) -> bool {
        let Some(timeline) = &self.timeline else {
            return false;
        };
        let wanted: Vec<_> = {
            let known = timeline.images.borrow();
            timeline.image_events().into_iter().filter(|id| !known.contains_key(id)).collect()
        };
        if !self.media.next_inline(self.active_scope.clone(), wanted) {
            return false;
        }
        if let Some(id) = self.media.event().cloned() {
            timeline.images.borrow_mut().insert(id.clone(), InlineImage::Loading);
            timeline.image_changed(&id);
        }
        true
    }

    /// Store the answer of the finished read under its message.
    pub(crate) fn update_media_image(&mut self) {
        let (Some(timeline), Some(id)) = (&self.timeline, self.media.event().cloned()) else {
            return;
        };
        let state = match self.media.take_preview() {
            Some(mut preview) => {
                // GPUI RenderImage expects BGRA, image decoder returns straight RGBA.
                for pixel in preview.rgba.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                image::RgbaImage::from_raw(preview.width, preview.height, preview.rgba)
                    .map_or(InlineImage::Failed, |buffer| {
                        InlineImage::Ready(Arc::new(gpui::RenderImage::new([image::Frame::new(buffer)])))
                    })
            }
            None => InlineImage::Failed,
        };
        timeline.images.borrow_mut().insert(id.clone(), state);
        timeline.image_changed(&id);
    }
}
