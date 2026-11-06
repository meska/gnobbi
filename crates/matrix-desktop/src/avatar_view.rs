//! Avatars of rooms and spaces: asked for in small batches from the worker, decoded there,
//! drawn as circles. No picture (yet) simply means the initials stay.
use gpui::{AnyElement, ObjectFit, RenderImage, StyledImage, img, prelude::*, px};
use matrix_engine::avatar::{AvatarPicture, AvatarSlot};
use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const MAX_IN_FLIGHT: usize = 16;
// A worker that ends mid-request never answers; forget the request after this long.
const GIVE_UP: Duration = Duration::from_secs(30);

pub(crate) struct Avatars {
    images: HashMap<String, Option<Arc<RenderImage>>>,
    in_flight: HashMap<String, Instant>,
    out: mpsc::Sender<AvatarPicture>,
    answers: mpsc::Receiver<AvatarPicture>,
}

impl Default for Avatars {
    fn default() -> Self {
        let (out, answers) = mpsc::channel();
        Self {
            images: HashMap::new(),
            in_flight: HashMap::new(),
            out,
            answers,
        }
    }
}

impl Avatars {
    pub(crate) fn get(&self, mxc: Option<&str>) -> Option<Arc<RenderImage>> {
        self.images.get(mxc?)?.clone()
    }

    /// One tick: take the answers, then ask for more of `wanted`. True when a picture arrived.
    pub(crate) fn drive<'a>(
        &mut self,
        slot: &AvatarSlot,
        wanted: impl Iterator<Item = &'a str>,
    ) -> bool {
        let mut arrived = false;
        while let Ok(picture) = self.answers.try_recv() {
            self.in_flight.remove(&picture.mxc);
            let image = picture.rgba.and_then(|(w, h, mut rgba)| {
                // GPUI RenderImage expects BGRA.
                for pixel in rgba.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                let buffer = image::RgbaImage::from_raw(w, h, rgba)?;
                Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
            });
            arrived |= image.is_some();
            self.images.insert(picture.mxc, image);
        }
        self.in_flight.retain(|_, at| at.elapsed() < GIVE_UP);
        let Ok(slot) = slot.try_lock() else {
            return arrived;
        };
        let Some(source) = slot.as_ref() else {
            return arrived;
        };
        for mxc in wanted {
            if self.in_flight.len() >= MAX_IN_FLIGHT {
                break;
            }
            if !self.images.contains_key(mxc) && !self.in_flight.contains_key(mxc) {
                self.in_flight.insert(mxc.to_owned(), Instant::now());
                source.request(mxc.to_owned(), self.out.clone());
            }
        }
        arrived
    }
}

/// The picture as a circle of `size` logical pixels; the 96 px thumbnail stays sharp up to 200 %.
pub(crate) fn round(image: Arc<RenderImage>, size: f32) -> AnyElement {
    img(image)
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .object_fit(ObjectFit::Cover)
        .into_any_element()
}

impl crate::Shell {
    /// Visible rows first, then every other room and space; true when a picture arrived.
    pub(crate) fn drive_avatars(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let rooms = self.sidebar.rooms();
        let visible = self.sidebar.visible_indices().iter().map(|&i| &rooms[i]);
        let senders = self.timeline.as_ref().map(|t| t.sender_avatar_mxcs()).unwrap_or_default();
        let listed = self
            .sidebar
            .spaces()
            .chain(visible)
            .chain(rooms)
            .filter_map(|room| room.info.avatar.as_deref());
        let mentions = self.composer.read(cx).mention_avatars();
        let wanted = mentions.iter().map(String::as_str).chain(senders.iter().map(String::as_str)).chain(listed);
        let arrived = self.avatars.drive(&self.routes.avatars, wanted);
        for mxc in mentions {
            if let Some(image) = self.avatars.get(Some(&mxc))
                && !self.composer.read(cx).has_picture(&mxc)
            {
                self.composer.update(cx, |c, cx| c.set_picture(mxc, image, cx));
            }
        }
        let mut changed = Vec::new();
        if let Some(timeline) = &self.timeline {
            let mut pictures = timeline.sender_pictures.borrow_mut();
            for mxc in &senders {
                if !pictures.contains_key(mxc)
                    && let Some(image) = self.avatars.get(Some(mxc))
                {
                    pictures.insert(mxc.clone(), image);
                    changed.push(mxc.clone());
                }
            }
            drop(pictures);
            changed.iter().for_each(|mxc| timeline.avatar_changed(mxc));
        }
        arrived
    }
}
