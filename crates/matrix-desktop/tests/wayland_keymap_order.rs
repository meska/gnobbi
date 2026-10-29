//! A compositor that sends `wl_keyboard.modifiers` before any `keymap` (seen with sway after
//! `wtype`) must not make the toolkit panic at startup. The fake compositor runs in this
//! process; GPUI runs in a child so a panic shows up as a failed exit status.
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

struct Fake;
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
        _: &mut Self,
        _: &Client,
        _: &WlSeat,
        req: wl_seat::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wl_seat::Request::GetKeyboard { id } = req {
            // The bug: modifiers arrive and no keymap is ever sent.
            d.init(id, ()).modifiers(1, 1, 0, 0, 0);
        }
    }
}

#[test]
fn modifiers_before_keymap_do_not_panic() {
    let dir = std::env::temp_dir().join(format!("gnobbi-fake-wl-{}", std::process::id()));
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
                display.dispatch_clients(&mut Fake).unwrap();
                display.flush_clients().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        })
    };
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "modifiers_before_keymap_child", "--nocapture"])
        .env_remove("DISPLAY")
        .env_remove("ZED_HEADLESS")
        .env("WAYLAND_DISPLAY", &path)
        .env("GNOBBI_FAKE_WL_CHILD", "1")
        .output()
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    thread.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
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
