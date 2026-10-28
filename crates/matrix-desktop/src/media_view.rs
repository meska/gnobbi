//! Pictures drawn inside message bubbles from bounded, already-decoded backend pixels. Never an image URL.
use crate::Shell;
use gpui::{AnyElement, div, img, prelude::*, px, rgb};
use matrix_desktop::DesignTokens;
use matrix_engine::media::MediaError;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) enum InlineImage {
    Loading,
    Ready(Arc<gpui::RenderImage>),
    Failed(Option<MediaError>),
}

pub(crate) fn failure_label(error: Option<MediaError>) -> &'static str {
    match error {
        Some(MediaError::TooLarge) => "Immagine troppo grande",
        _ => "Immagine non disponibile",
    }
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
        Some(InlineImage::Failed(error)) => div()
            .w(px(200.))
            .h(px(60.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .bg(rgb(t.timeline))
            .text_size(px(12.))
            .text_color(rgb(t.error))
            .child(failure_label(error))
            .into_any_element(),
        _ => div()
            .w(px(200.))
            .h(px(150.))
            .rounded_md()
            .bg(rgb(t.timeline))
            .into_any_element(),
    }
}

/// Full-area dark scrim: the cached thumbnail until the original is ready, then the original (same
/// frame, no flicker). A click closes it; a failed original shows one row with «Riprova».
pub(crate) fn large_view(
    thumbnail: Arc<gpui::RenderImage>,
    original: Option<InlineImage>,
    t: DesignTokens,
    close: impl Fn(&mut gpui::Window) + 'static,
    retry: impl Fn(&mut gpui::Window) + 'static,
) -> AnyElement {
    let (image, failure) = match original {
        Some(InlineImage::Ready(image)) => (image, None),
        Some(InlineImage::Failed(error)) => (thumbnail, Some(error)),
        _ => (thumbnail, None),
    };
    div()
        .absolute()
        .inset_0()
        .child(
            div()
                .id("large-image")
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .bg(gpui::rgba(0x000000d9))
                .cursor_pointer()
                .on_click(move |_, window, _| close(window))
                .child(
                    img(image)
                        .max_w_full()
                        .max_h_full()
                        .object_fit(gpui::ObjectFit::Contain),
                ),
        )
        .children(failure.map(|error| {
            div()
                .occlude()
                .absolute()
                .bottom_4()
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(t.timeline))
                        .text_size(px(12.))
                        .text_color(rgb(t.error))
                        .child(failure_label(error))
                        .child(
                            div()
                                .id("large-image-retry")
                                .cursor_pointer()
                                .text_color(rgb(t.accent))
                                .on_click(move |_, window, _| retry(window))
                                .child("Riprova"),
                        ),
                )
        }))
        .into_any_element()
}

impl Shell {
    /// Ask for the next image of the open room, one read at a time; true when the view changed.
    pub(crate) fn drive_inline_images(&mut self) -> bool {
        let Some(timeline) = &self.timeline else {
            return false;
        };
        let mut changed = false;
        if let Some(id) = timeline.want_original.borrow_mut().take() {
            self.media.request_original(self.active_scope.clone(), id);
            changed = true;
        } else if timeline.viewing().is_none() && timeline.original.borrow().is_some() {
            // Closed: drop the full-resolution picture and stop its read.
            self.media.cancel_original();
            *timeline.original.borrow_mut() = None;
        }
        let wanted: Vec<_> = {
            let known = timeline.images.borrow();
            timeline
                .image_events()
                .into_iter()
                .filter(|id| !known.contains_key(id))
                .collect()
        };
        if !self.media.next_inline(self.active_scope.clone(), wanted) {
            return changed;
        }
        if let Some(id) = self.media.event().cloned() {
            timeline
                .images
                .borrow_mut()
                .insert(id.clone(), InlineImage::Loading);
            timeline.image_changed(&id);
        }
        true
    }

    /// Store the answer of the finished read under its message.
    pub(crate) fn update_media_image(&mut self) {
        if let (Some(timeline), Some((id, result))) = (&self.timeline, self.media.take_original()) {
            let state = match result {
                Ok(preview) => render_image(preview),
                Err(error) => InlineImage::Failed(Some(error)),
            };
            if timeline.viewing().as_ref() == Some(&id) {
                *timeline.original.borrow_mut() = Some((id, state));
            }
            return;
        }
        let (Some(timeline), Some(id)) = (&self.timeline, self.media.event().cloned()) else {
            return;
        };
        let error = self.media.error();
        let state = match self.media.take_preview() {
            Some(preview) => render_image(preview),
            None => InlineImage::Failed(error),
        };
        timeline.images.borrow_mut().insert(id.clone(), state);
        timeline.image_changed(&id);
    }
}

fn render_image(mut preview: matrix_engine::media::Preview) -> InlineImage {
    // GPUI RenderImage expects BGRA, image decoder returns straight RGBA.
    for pixel in preview.rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    image::RgbaImage::from_raw(preview.width, preview.height, preview.rgba).map_or(
        InlineImage::Failed(None),
        |buffer| {
            InlineImage::Ready(Arc::new(gpui::RenderImage::new([image::Frame::new(
                buffer,
            )])))
        },
    )
}

#[cfg(test)]
mod tests {
    use matrix_engine::media::MediaError;

    #[test]
    fn only_oversized_images_say_too_large() {
        assert_eq!(
            super::failure_label(Some(MediaError::TooLarge)),
            "Immagine troppo grande"
        );
        for error in [
            None,
            Some(MediaError::Failed),
            Some(MediaError::InvalidImage),
            Some(MediaError::UnsupportedType),
        ] {
            assert_eq!(super::failure_label(error), "Immagine non disponibile");
        }
    }
}
