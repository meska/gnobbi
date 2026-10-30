//! Pictures drawn inside message bubbles from bounded, already-decoded backend pixels. Never an image URL.
use crate::Shell;
use gpui::{AnyElement, div, img, prelude::*, px, rgb};
use matrix_desktop::DesignTokens;
use matrix_engine::media::MediaError;
use std::sync::Arc;

// Exception: the media viewer is black in every theme, like any photo viewer.
const VIEWER_CHIP: u32 = 0x000000a6;
const VIEWER_BAR: u32 = 0x000000d9;

#[derive(Clone)]
pub(crate) enum InlineImage {
    Loading,
    Ready(Arc<gpui::RenderImage>),
    Failed(Option<MediaError>),
}

pub(crate) fn failure_label(error: Option<MediaError>, video: bool) -> &'static str {
    match (video, error) {
        (false, Some(MediaError::TooLarge)) => "Immagine troppo grande",
        (false, _) => "Immagine non disponibile",
        (true, Some(MediaError::TooLarge)) => "File troppo grande",
        (true, _) => "Riproduzione non disponibile",
    }
}

/// Over a cap the thumbnail stays as the largest allowed version; asking again cannot change that.
pub(crate) fn retryable(error: Option<MediaError>) -> bool {
    error != Some(MediaError::TooLarge)
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
            .child(failure_label(error, false))
            .into_any_element(),
        _ => div()
            .w(px(200.))
            .h(px(150.))
            .rounded_md()
            .bg(rgb(t.timeline))
            .into_any_element(),
    }
}

pub(crate) type SaveClick = std::rc::Rc<dyn Fn(&mut gpui::Window, &mut gpui::App)>;

/// «Scarica» at the bubble's bottom right, icon only: ↓ idle, … while saving (a click stops it).
/// Its own focusable element, never part of the picture, so a click here cannot open the large view.
pub(crate) fn save_icon(
    index: usize,
    running: bool,
    t: DesignTokens,
    click: SaveClick,
    painted: impl Fn(gpui::Bounds<gpui::Pixels>) + 'static,
) -> AnyElement {
    div()
        .id(("save-attachment", index))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(px(24.))
        .rounded_sm()
        .text_size(px(14.))
        .text_color(rgb(t.accent))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(t.border)))
        .focusable()
        .tab_stop(true)
        .focus(|s| s.bg(rgb(t.border)))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            click(window, cx)
        })
        .child(if running { "…" } else { "↓" })
        .child(gpui::canvas(|_, _, _| (), move |bounds, _, _, _| painted(bounds)).absolute().inset_0())
        .into_any_element()
}

/// One line under the bubble footer: what went wrong and, unless it cannot change, «Riprova».
pub(crate) fn save_failure(index: usize, error: MediaError, t: DesignTokens, retry: SaveClick) -> AnyElement {
    div()
        .flex()
        .justify_end()
        .gap_2()
        .text_size(px(12.))
        .text_color(rgb(t.error))
        .child(matrix_desktop::save_failure_label(error))
        .when(matrix_desktop::save_retryable(error), |d| {
            d.child(
                div()
                    .id(("save-retry", index))
                    .cursor_pointer()
                    .text_color(rgb(t.accent))
                    .on_click(move |_, window, cx| retry(window, cx))
                    .child("Riprova"),
            )
        })
        .into_any_element()
}

/// Painted parts of a video bubble, by name, so a test can see what reached the screen.
pub(crate) type PaintedParts = std::rc::Rc<dyn Fn(&'static str, gpui::Bounds<gpui::Pixels>)>;

/// Video bubble: the thumbnail, or a neutral box when there is none, with a play symbol over it and
/// the duration and size in the corners. Nothing plays and no text explains it.
pub(crate) fn video_picture(
    state: Option<InlineImage>,
    duration: Option<String>,
    size: Option<String>,
    t: DesignTokens,
    painted: PaintedParts,
) -> AnyElement {
    let base = match state {
        Some(ready @ InlineImage::Ready(_)) => inline_picture(Some(ready), t),
        _ => inline_picture(None, t),
    };
    let probe = {
        let painted = painted.clone();
        move |part: &'static str| {
            let painted = painted.clone();
            gpui::canvas(|_, _, _| (), move |bounds, _, _, _| painted(part, bounds))
                .absolute()
                .inset_0()
        }
    };
    let chip = |part: &'static str, text: String| {
        div()
            .relative()
            .px_1()
            .rounded_sm()
            .bg(gpui::rgba(VIEWER_CHIP))
            .text_size(px(12.))
            .text_color(gpui::white())
            .child(text)
            .child(probe(part))
    };
    // A flex row gives the bubble the picture's own width instead of the row's.
    div()
        .flex()
        .child(
            div()
                .relative()
                .child(base)
                .child(probe("picture"))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .relative()
                                .size(px(44.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(gpui::rgba(VIEWER_CHIP))
                                .text_size(px(20.))
                                .text_color(gpui::white())
                                .child("▶")
                                .child(probe("play")),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .p_1()
                        .flex()
                        .justify_between()
                        .children(duration.map(|d| chip("duration", d)))
                        .children(size.map(|s| chip("size", s))),
                ),
        )
        .into_any_element()
}

/// Full-area dark scrim: the cached thumbnail until the original is ready, then the original (same
/// frame, no flicker). A click closes it; a failed original shows one row with «Riprova».
/// For a video there may be no thumbnail, and `original` only carries a failed download.
pub(crate) fn large_view(
    thumbnail: Option<Arc<gpui::RenderImage>>,
    original: Option<InlineImage>,
    video: bool,
    t: DesignTokens,
    close: impl Fn(&mut gpui::Window) + 'static,
    retry: impl Fn(&mut gpui::Window) + 'static,
) -> AnyElement {
    let (image, failure) = match original {
        Some(InlineImage::Ready(image)) => (Some(image), None),
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
                .bg(gpui::rgba(VIEWER_BAR))
                .cursor_pointer()
                .on_click(move |_, window, _| close(window))
                .children(image.map(|image| {
                    img(image)
                        .max_w_full()
                        .max_h_full()
                        .object_fit(gpui::ObjectFit::Contain)
                })),
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
                        .child(failure_label(error, video))
                        .children(retryable(error).then(|| {
                            div()
                                .id("large-image-retry")
                                .cursor_pointer()
                                .text_color(rgb(t.accent))
                                .on_click(move |_, window, _| retry(window))
                                .child("Riprova")
                        })),
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
        } else if let Some(id) = timeline.want_video.borrow_mut().take() {
            self.media.request_video(self.active_scope.clone(), id);
            changed = true;
        } else if timeline.viewing().is_none()
            && (timeline.original.borrow().is_some() || self.media.original_pending())
        {
            // Closed: drop the full-resolution picture and stop its read.
            self.media.cancel_original();
            *timeline.original.borrow_mut() = None;
        }
        self.media.mark_videos(timeline.video_events());
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

    /// «Scarica»: stop a save on request, and turn a click into the file picker, then into the save.
    pub(crate) fn drive_saves(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(timeline) = &self.timeline else {
            return;
        };
        if timeline.stop_save.take() {
            self.media.cancel_save();
        }
        let Some((id, name)) = timeline.want_save.borrow_mut().take() else {
            return;
        };
        let picked = cx.prompt_for_new_path(&matrix_desktop::save_start_dir(), Some(&name));
        let scope = self.active_scope.clone();
        cx.spawn(async move |this, cx| {
            let path = picked.await.ok().and_then(Result::ok).flatten();
            let _ = this.update(cx, |this, cx| {
                let Some(timeline) = &this.timeline else {
                    return;
                };
                // Stopped while the picker was open, or the room changed: nothing to save.
                let wanted = timeline.saves.borrow().state(&id) == Some(matrix_desktop::SaveState::Running)
                    && scope == this.active_scope;
                let started = wanted
                    && path.is_some_and(|path| this.media.request_save(scope, id.clone(), path));
                if !started && wanted {
                    timeline.saves.borrow_mut().cancel(&id);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Store the answer of the finished read under its message.
    pub(crate) fn update_media_image(&mut self) {
        if let (Some(timeline), Some((id, result))) = (&self.timeline, self.media.take_saved()) {
            timeline.finish_save(&id, result);
            return;
        }
        if let (Some(timeline), Some((id, result))) = (&self.timeline, self.media.take_video()) {
            let opened = result.and_then(|file| {
                crate::video_player::open(file.path()).map_err(|_| MediaError::Failed)?;
                Ok(file)
            });
            match opened {
                Ok(file) => {
                    // The player has its own window: the large view has done its job.
                    self.media.hold_playing(file);
                    if timeline.viewing().as_ref() == Some(&id) {
                        *timeline.viewing.borrow_mut() = None;
                    }
                }
                Err(error) if timeline.viewing().as_ref() == Some(&id) => {
                    *timeline.original.borrow_mut() = Some((id, InlineImage::Failed(Some(error))));
                }
                Err(_) => {}
            }
            return;
        }
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
    fn over_the_cap_has_no_retry_other_errors_do() {
        assert!(!super::retryable(Some(MediaError::TooLarge)));
        assert!(super::retryable(Some(MediaError::Failed)));
        assert!(super::retryable(None));
    }

    #[test]
    fn only_oversized_images_say_too_large() {
        assert_eq!(
            super::failure_label(Some(MediaError::TooLarge), false),
            "Immagine troppo grande"
        );
        assert_eq!(
            super::failure_label(Some(MediaError::TooLarge), true),
            "File troppo grande"
        );
        assert_eq!(super::failure_label(None, true), "Riproduzione non disponibile");
        for error in [
            None,
            Some(MediaError::Failed),
            Some(MediaError::InvalidImage),
            Some(MediaError::UnsupportedType),
        ] {
            assert_eq!(
                super::failure_label(error, false),
                "Immagine non disponibile"
            );
        }
    }
}
