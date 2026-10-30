//! A compositor that sends `wl_keyboard.modifiers` before any `keymap` (seen with sway after
//! `wtype`) must not make the toolkit panic at startup. The fake compositor runs in this
//! process; GPUI runs in a child so a panic shows up as a failed exit status.
//!
//! Run (Linux only, headless: no sway, no X11, no VM; DISPLAY is removed for the child):
//! `cargo test --locked -p matrix-desktop --features desktop --test wayland_keymap_order`
//! On gpui 0.2.2 both cases fail with the child panic at `wayland/client.rs:1267`
//! (`Option::unwrap()` on a `None` keymap_state); they pass once that is fixed.
#![cfg(all(target_os = "linux", feature = "desktop"))]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use wayland_protocols::xdg::shell::server::xdg_wm_base::XdgWmBase;
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, ListeningSocket, New,
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::{
        wl_buffer::{self, WlBuffer},
        wl_compositor::{self, WlCompositor},
        wl_keyboard::{self, WlKeyboard},
        wl_seat::{self, WlSeat},
        wl_shm::{self, WlShm},
        wl_shm_pool::{self, WlShmPool},
        wl_surface::{self, WlSurface},
    },
};

struct Fake {
    keymap_late: bool,
}
struct NoData;
impl ClientData for NoData {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

macro_rules! plain_global {
    ($iface:ty) => {
        impl GlobalDispatch<$iface, ()> for Fake {
            fn bind(
                _: &mut Self,
                _: &DisplayHandle,
                _: &Client,
                r: New<$iface>,
                _: &(),
                d: &mut DataInit<'_, Self>,
            ) {
                d.init(r, ());
            }
        }
        impl Dispatch<$iface, ()> for Fake {
            fn request(
                _: &mut Self,
                _: &Client,
                _: &$iface,
                _: <$iface as wayland_server::Resource>::Request,
                _: &(),
                _: &DisplayHandle,
                _: &mut DataInit<'_, Self>,
            ) {
            }
        }
    };
}
impl Dispatch<WlSurface, ()> for Fake {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlSurface,
        _: wl_surface::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl GlobalDispatch<WlCompositor, ()> for Fake {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<WlCompositor>,
        _: &(),
        d: &mut DataInit<'_, Self>,
    ) {
        d.init(r, ());
    }
}
impl Dispatch<WlCompositor, ()> for Fake {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlCompositor,
        req: wl_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wl_compositor::Request::CreateSurface { id } = req {
            d.init(id, ());
        }
    }
}
impl GlobalDispatch<WlShm, ()> for Fake {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<WlShm>,
        _: &(),
        d: &mut DataInit<'_, Self>,
    ) {
        d.init(r, ());
    }
}
impl Dispatch<WlShm, ()> for Fake {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlShm,
        req: wl_shm::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, .. } = req {
            d.init(id, ());
        }
    }
}
impl Dispatch<WlShmPool, ()> for Fake {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlShmPool,
        req: wl_shm_pool::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm_pool::Request::CreateBuffer { id, .. } = req {
            d.init(id, ());
        }
    }
}
impl Dispatch<WlBuffer, ()> for Fake {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlBuffer,
        _: wl_buffer::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
plain_global!(XdgWmBase);
impl Dispatch<WlKeyboard, ()> for Fake {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlKeyboard,
        _: wl_keyboard::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl GlobalDispatch<WlSeat, ()> for Fake {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<WlSeat>,
        _: &(),
        d: &mut DataInit<'_, Self>,
    ) {
        d.init(r, ()).capabilities(wl_seat::Capability::Keyboard);
    }
}
impl Dispatch<WlSeat, ()> for Fake {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &WlSeat,
        req: wl_seat::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wl_seat::Request::GetKeyboard { id } = req {
            // The bug: modifiers arrive before the keymap (or the keymap never comes).
            let kb = d.init(id, ());
            kb.modifiers(1, 1, 0, 0, 0);
            if state.keymap_late {
                let f = std::fs::File::options().read(true).write(true).create(true).truncate(true).open(
                    std::env::temp_dir().join(format!("gnobbi-km-{}", std::process::id())),
                )
                .unwrap();
                let km = b"xkb_keymap { xkb_keycodes { include \"evdev\" }; xkb_types { include \"complete\" }; xkb_compat { include \"complete\" }; xkb_symbols { include \"pc+us\" }; };\0";
                std::io::Write::write_all(&mut &f, km).unwrap();
                kb.keymap(
                    wl_keyboard::KeymapFormat::XkbV1,
                    std::os::fd::AsFd::as_fd(&f),
                    km.len() as u32,
                );
            }
        }
    }
}

/// No keymap ever: the keyboard only gets `modifiers`.
#[test]
fn modifiers_without_keymap_do_not_panic() {
    run_case(false);
}

/// `modifiers` first, the keymap right after (the order sway uses after `wtype`).
#[test]
fn modifiers_before_late_keymap_do_not_panic() {
    run_case(true);
}

fn run_case(keymap_late: bool) {
    let dir = std::env::temp_dir().join(format!(
        "gnobbi-fake-wl-{}-{}",
        std::process::id(),
        keymap_late
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("wayland-fake");
    let socket = ListeningSocket::bind_absolute(path.clone()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut display = Display::<Fake>::new().unwrap();
            let mut h = display.handle();
            h.create_global::<Fake, WlCompositor, _>(6, ());
            h.create_global::<Fake, WlShm, _>(1, ());
            h.create_global::<Fake, XdgWmBase, _>(2, ());
            h.create_global::<Fake, WlSeat, _>(5, ());
            while !stop.load(Ordering::Relaxed) {
                if let Some(s) = socket.accept().unwrap() {
                    h.insert_client(s, Arc::new(NoData)).unwrap();
                }
                display.dispatch_clients(&mut Fake { keymap_late }).unwrap();
                display.flush_clients().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        })
    };
    let log_path = dir.join("child.log");
    let log_file = std::fs::File::create(&log_path).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "modifiers_before_keymap_child", "--nocapture"])
        .env_remove("DISPLAY")
        .env_remove("ZED_HEADLESS")
        .env("WAYLAND_DISPLAY", &path)
        .env("GNOBBI_FAKE_WL_CHILD", "1")
        .stdout(log_file.try_clone().unwrap())
        .stderr(log_file.try_clone().unwrap())
        .spawn()
        .unwrap();
    // A hang must fail the test, not block it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break Some(s);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    stop.store(true, Ordering::Relaxed);
    thread.join().unwrap();
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(
        std::env::temp_dir().join(format!("gnobbi-km-{}", std::process::id())),
    );
    // Guards against a green run where the child never reached the Wayland client at all.
    assert!(
        log.contains("modifiers_before_keymap_child"),
        "child did not run: {log}"
    );
    assert!(
        status.is_some_and(|s| s.success()),
        "GPUI panicked or hung (10 s) (wayland/client.rs keymap_state unwrap):\n{log}"
    );
}

#[test]
fn modifiers_before_keymap_child() {
    if std::env::var_os("GNOBBI_FAKE_WL_CHILD").is_none() {
        return;
    }
    gpui::Application::new().run(|cx| {
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            executor.timer(std::time::Duration::from_millis(800)).await;
            cx.update(|cx| cx.quit()).unwrap();
        })
        .detach();
    });
}
