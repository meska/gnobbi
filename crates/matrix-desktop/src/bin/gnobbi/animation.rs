//! Playback of animated pictures (GIF, WebP, APNG). The texture stays the one of the existing caches (the timeline's
//! `pictures`, the sticker tab's `avatars`): a step uploads the frame due into it. No frame is asked for while the
//! picture is not painted, so an idle window with no animated picture on screen costs nothing.
use eframe::egui;
use matrix_engine::anim::{Frames, frame_at};
use std::{cell::RefCell, collections::{HashMap, HashSet, VecDeque}, time::{Duration, Instant}};

/// Decoded frames kept in all; the oldest animation stops (stays on the frame it shows) past this.
const MAX_BYTES: usize = 96 * 1024 * 1024;

struct Player {
    frames: Frames,
    started: Instant,
    shown: usize,
}

#[derive(Default)]
struct State {
    players: HashMap<String, Player>,
    order: VecDeque<String>,
    bytes: usize,
    wanted: Option<String>,
    asked: HashSet<String>,
}

thread_local! {
    // UI thread only.
    static STATE: RefCell<State> = RefCell::default();
}

/// Registers the frames of picture `key` (an event id in the chat, an mxc in the sticker tab).
pub fn insert(key: String, frames: Frames) {
    STATE.with_borrow_mut(|s| {
        s.bytes += frames.bytes();
        if let Some(old) = s.players.insert(key.clone(), Player { frames, started: Instant::now(), shown: 0 }) {
            s.bytes -= old.frames.bytes();
        } else {
            s.order.push_back(key);
        }
        while s.bytes > MAX_BYTES && s.order.len() > 1 {
            if let Some(p) = s.order.pop_front().and_then(|k| s.players.remove(&k)) {
                s.bytes -= p.frames.bytes();
            }
        }
    });
}

/// Forgets every animation (a room change drops the pictures too).
pub fn clear() {
    STATE.with_borrow_mut(|s| *s = State::default());
}

/// Sticker tab: the pointer is on `key`; asks once for its frames unless motion is reduced or the picture is still.
pub fn hover(key: &str) {
    if crate::mobile::reduce_motion() {
        return;
    }
    STATE.with_borrow_mut(|s| {
        if !s.players.contains_key(key) && s.asked.insert(key.to_owned()) {
            s.wanted = Some(key.to_owned());
        }
    });
}

/// The picture whose frames the session should fetch next.
pub fn take_wanted() -> Option<String> {
    STATE.with_borrow_mut(|s| s.wanted.take())
}

/// Call each time `key` is painted with `tex`: with `play` it shows the frame due now and asks for the next repaint when
/// that frame ends (true then); without it (or with «reduce motion», or in the background) it rests on the first frame.
pub fn step(ctx: &egui::Context, key: &str, tex: &egui::TextureHandle, play: bool) -> bool {
    let play = play && !crate::mobile::reduce_motion() && !crate::suspend::is_suspended();
    let next = STATE.with_borrow_mut(|s| {
        let p = s.players.get_mut(key)?;
        if !play {
            p.started = Instant::now();
        }
        let (due, wait) = if play { frame_at(&p.frames.delays_ms, p.started.elapsed().as_millis() as u64) } else { (0, 0) };
        if due != p.shown {
            p.shown = due;
            let image = egui::ColorImage::from_rgba_unmultiplied([p.frames.width as usize, p.frames.height as usize], &p.frames.rgba[due]);
            tex.clone().set(image, Default::default());
        }
        play.then_some(wait)
    });
    if let Some(wait) = next {
        ctx.request_repaint_after(Duration::from_millis(u64::from(wait.max(1))));
    }
    next.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_frames() -> Frames {
        Frames { width: 1, height: 1, rgba: vec![vec![255, 0, 0, 255], vec![0, 255, 0, 255]], delays_ms: vec![20, 20] }
    }

    #[test]
    fn repaint_only_while_playing_and_oldest_goes_over_the_cap() {
        clear();
        let ctx = egui::Context::default();
        let tex = ctx.load_texture("t", egui::ColorImage::example(), Default::default());
        insert("a".into(), two_frames());
        assert!(!step(&ctx, "a", &tex, false));
        assert!(step(&ctx, "a", &tex, true));
        assert!(!step(&ctx, "unknown", &tex, true));
        hover("x");
        assert_eq!(take_wanted().as_deref(), Some("x"));
        hover("x");
        assert_eq!(take_wanted(), None);
        clear();
    }
}
