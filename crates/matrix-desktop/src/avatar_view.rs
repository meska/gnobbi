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
// shortcut: fixed wait after a transient failure, no growing backoff; upgrade if servers get hammered.
const RETRY_AFTER: Duration = Duration::from_secs(30);

pub(crate) struct Avatars {
    images: HashMap<String, Option<Arc<RenderImage>>>,
    in_flight: HashMap<String, Instant>,
    // Transient failures: when each was answered, so it is asked again after `retry_after`.
    failed: HashMap<String, Instant>,
    retry_after: Duration,
    out: mpsc::Sender<AvatarPicture>,
    answers: mpsc::Receiver<AvatarPicture>,
}

impl Default for Avatars {
    fn default() -> Self {
        let (out, answers) = mpsc::channel();
        Self {
            images: HashMap::new(),
            in_flight: HashMap::new(),
            failed: HashMap::new(),
            retry_after: RETRY_AFTER,
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
            if image.is_none() && picture.retry {
                self.failed.insert(picture.mxc, Instant::now());
                continue;
            }
            self.failed.remove(&picture.mxc);
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
            let waiting = self
                .failed
                .get(mxc)
                .is_some_and(|at| at.elapsed() < self.retry_after);
            if !self.images.contains_key(mxc) && !self.in_flight.contains_key(mxc) && !waiting {
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
        let senders = self
            .timeline
            .as_ref()
            .map(|t| t.sender_avatar_mxcs())
            .unwrap_or_default();
        let listed = self
            .sidebar
            .spaces()
            .chain(visible)
            .chain(rooms)
            .filter_map(|room| room.info.avatar.as_deref());
        let people = self.people_avatars();
        let own = self.verification.profile_avatar();
        let found = self.new_chat_avatars();
        let mentions = self.composer.read(cx).mention_avatars();
        let wanted = mentions
            .iter()
            .map(String::as_str)
            .chain(own)
            .chain(people.iter().map(String::as_str))
            .chain(found.iter().map(String::as_str))
            .chain(senders.iter().map(String::as_str))
            .chain(listed);
        let mut arrived = self.avatars.drive(&self.routes.avatars, wanted);
        // Drawer rows: each parked account asks its own session for its own picture.
        for state in self.parked.values() {
            let own = state.verification.profile_avatar();
            arrived |= self.avatars.drive(&state.routes.avatars, own.into_iter());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_engine::avatar::AvatarSource;
    use std::sync::Mutex;

    /// Answers each request with the next scripted outcome: None = transient failure.
    struct Script(Mutex<Vec<bool>>, Arc<Mutex<usize>>);
    impl AvatarSource for Script {
        fn request(&self, mxc: String, out: mpsc::Sender<AvatarPicture>) {
            *self.1.lock().unwrap() += 1;
            let ok = self.0.lock().unwrap().remove(0);
            let rgba = ok.then(|| (1, 1, vec![0, 0, 0, 255]));
            let _ = out.send(AvatarPicture {
                mxc,
                rgba,
                retry: !ok,
            });
        }
    }

    #[test]
    fn transient_failure_is_asked_again_after_the_wait_and_success_sticks() {
        let calls = Arc::new(Mutex::new(0));
        let slot: AvatarSlot = Arc::new(Mutex::new(Some(Box::new(Script(
            Mutex::new(vec![false, true]),
            calls.clone(),
        )) as Box<dyn AvatarSource>)));
        let mut avatars = Avatars::default();
        avatars.drive(&slot, ["mxc://s/a"].into_iter()); // asked, fails
        assert!(!avatars.drive(&slot, ["mxc://s/a"].into_iter())); // answer taken, now waiting
        assert_eq!(
            (
                *calls.lock().unwrap(),
                avatars.get(Some("mxc://s/a")).is_none()
            ),
            (1, true)
        );
        avatars.drive(&slot, ["mxc://s/a"].into_iter());
        assert_eq!(*calls.lock().unwrap(), 1, "still inside the wait");
        avatars.retry_after = Duration::ZERO;
        avatars.drive(&slot, ["mxc://s/a"].into_iter()); // asked again
        assert!(avatars.drive(&slot, ["mxc://s/a"].into_iter())); // succeeds
        assert!(avatars.get(Some("mxc://s/a")).is_some());
        avatars.drive(&slot, ["mxc://s/a"].into_iter());
        assert_eq!(*calls.lock().unwrap(), 2, "a picture is never asked again");
    }

    #[test]
    fn final_failure_is_not_retried() {
        struct Gone(Arc<Mutex<usize>>);
        impl AvatarSource for Gone {
            fn request(&self, mxc: String, out: mpsc::Sender<AvatarPicture>) {
                *self.0.lock().unwrap() += 1;
                let _ = out.send(AvatarPicture {
                    mxc,
                    rgba: None,
                    retry: false,
                });
            }
        }
        let calls = Arc::new(Mutex::new(0));
        let slot: AvatarSlot = Arc::new(Mutex::new(Some(
            Box::new(Gone(calls.clone())) as Box<dyn AvatarSource>
        )));
        let mut avatars = Avatars {
            retry_after: Duration::ZERO,
            ..Avatars::default()
        };
        for _ in 0..4 {
            avatars.drive(&slot, ["mxc://s/a"].into_iter());
        }
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn no_picture_asks_nothing_and_a_new_one_later_is_asked_and_shown() {
        let calls = Arc::new(Mutex::new(0));
        let slot: AvatarSlot = Arc::new(Mutex::new(Some(Box::new(Script(
            Mutex::new(vec![true, true]),
            calls.clone(),
        )) as Box<dyn AvatarSource>)));
        let mut avatars = Avatars::default();
        avatars.drive(&slot, std::iter::empty()); // account without picture
        assert_eq!(*calls.lock().unwrap(), 0);
        avatars.drive(&slot, ["mxc://s/old"].into_iter());
        avatars.drive(&slot, ["mxc://s/old"].into_iter());
        avatars.drive(&slot, ["mxc://s/new"].into_iter()); // profile now points elsewhere
        assert!(avatars.drive(&slot, ["mxc://s/new"].into_iter()));
        assert!(avatars.get(Some("mxc://s/new")).is_some());
        assert_eq!(*calls.lock().unwrap(), 2);
    }
}
