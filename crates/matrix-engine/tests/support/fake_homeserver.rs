//! Scriptable fake homeserver for sync-flapping tests: speaks just enough Matrix (login, versions,
//! whoami, sliding sync) over loopback HTTP/1.1. Run it in the same current-thread runtime as the
//! client with `start_paused(true)`: every delay below is virtual, so the tests are instant and
//! deterministic. Only paths and statuses are logged, never tokens or bodies.
#![allow(dead_code)]
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
    time::{Instant, sleep},
};

/// What the sliding-sync endpoint does with requests that carry a `pos` (the long-polls).
#[derive(Clone, Copy, Debug)]
pub enum SyncMode {
    /// Hold for the client's `timeout`, then answer an empty delta.
    Normal,
    /// A proxy with an idle timeout: hold this long, then close the socket without answering.
    DropAfter(Duration),
    /// A real server error, answered at once.
    Status(u16),
}

struct State {
    sync: SyncMode,
    /// Applies to the next whoami only (one slow answer, then back to instant).
    whoami_delay_once: Option<Duration>,
    /// Network down: every new connection is closed unanswered, held long-polls are cut.
    outage: bool,
    /// Mute server: TCP stays open but NOTHING is ever answered (held forever),
    /// including requests already in flight when it turns on.
    mute: bool,
    pos: u64,
    log: Vec<(Duration, String)>,
}

#[derive(Clone)]
pub struct Fake {
    pub url: String,
    state: Arc<Mutex<State>>,
    cut: Arc<Notify>,
    t0: Instant,
}

impl Fake {
    pub async fn start() -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Fake {
            url: format!("http://{}", listener.local_addr().unwrap()),
            state: Arc::new(Mutex::new(State {
                sync: SyncMode::Normal,
                whoami_delay_once: None,
                outage: false,
                mute: false,
                pos: 0,
                log: Vec::new(),
            })),
            cut: Arc::new(Notify::new()),
            t0: Instant::now(),
        };
        let server = fake.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(server.clone().serve(socket));
            }
        });
        fake
    }

    pub fn set_sync(&self, mode: SyncMode) {
        self.state.lock().unwrap().sync = mode;
    }
    pub fn delay_next_whoami(&self, by: Duration) {
        self.state.lock().unwrap().whoami_delay_once = Some(by);
    }
    pub fn set_outage(&self, down: bool) {
        self.state.lock().unwrap().outage = down;
        if down {
            self.cut.notify_waiters();
        }
    }
    /// Mute server from now on: accept, read, never answer, never close.
    pub fn set_mute(&self, on: bool) {
        self.state.lock().unwrap().mute = on;
    }
    /// Requests seen so far as (virtual time since start, "METHOD kind -> outcome").
    pub fn log(&self) -> Vec<(Duration, String)> {
        self.state.lock().unwrap().log.clone()
    }
    pub fn count(&self, kind: &str) -> usize {
        self.log().iter().filter(|(_, l)| l.contains(kind)).count()
    }

    fn note(&self, line: String) {
        let at = self.t0.elapsed();
        self.state.lock().unwrap().log.push((at, line));
    }

    async fn serve(self, mut socket: TcpStream) {
        let Some((method, path)) = read_request(&mut socket).await else {
            return;
        };
        if self.state.lock().unwrap().outage {
            self.note(format!("{method} {} -> dropped (outage)", kind(&path)));
            return;
        }
        if self.state.lock().unwrap().mute {
            self.note(format!("{method} {} -> MUTED (held, no answer)", kind(&path)));
            sleep(Duration::from_secs(86400)).await;
            return;
        }
        let reply = match kind(&path) {
            "login" => Some((
                200,
                r#"{"user_id":"@a:fake.test","access_token":"t","device_id":"D"}"#.to_owned(),
            )),
            "versions" => Some((
                200,
                r#"{"versions":["v1.11"],"unstable_features":{}}"#.to_owned(),
            )),
            "whoami" => {
                let delay = self.state.lock().unwrap().whoami_delay_once.take();
                if let Some(delay) = delay {
                    self.note(format!("whoami -> held {}s", delay.as_secs()));
                    sleep(delay).await;
                }
                Some((200, r#"{"user_id":"@a:fake.test","device_id":"D"}"#.to_owned()))
            }
            "sliding" => self.sliding(&path).await,
            _ => Some((404, r#"{"errcode":"M_UNRECOGNIZED","error":"x"}"#.to_owned())),
        };
        match reply {
            Some((status, body)) => {
                self.note(format!("{method} {} -> {status}", kind(&path)));
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                // The client may have given up already (its own timeout): ignore.
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
            None => {} // socket dropped: the client sees a closed connection
        }
    }

    /// None = close the connection without answering.
    async fn sliding(&self, path: &str) -> Option<(u16, String)> {
        if self.state.lock().unwrap().mute {
            // Mute also swallows long-polls already in flight: held, never answered.
            self.note("sliding -> MUTED (held, no answer)".to_owned());
            sleep(Duration::from_secs(86400)).await;
            return None;
        }
        let mode = self.state.lock().unwrap().sync;
        let long_poll = path.contains("pos=");
        let timeout = param(path, "timeout").map_or(Duration::ZERO, Duration::from_millis);
        match mode {
            SyncMode::Status(code) => {
                return Some((code, r#"{"errcode":"M_UNKNOWN","error":"boom"}"#.to_owned()));
            }
            SyncMode::DropAfter(wait) if long_poll => {
                self.note(format!("sliding -> held {}s then closed", wait.as_secs()));
                tokio::select! { _ = sleep(wait) => {}, _ = self.cut.notified() => {} }
                return None;
            }
            _ => {}
        }
        if long_poll {
            tokio::select! { _ = sleep(timeout) => {}, _ = self.cut.notified() => return None }
        }
        let pos = {
            let mut state = self.state.lock().unwrap();
            state.pos += 1;
            state.pos
        };
        Some((
            200,
            format!(r#"{{"pos":"{pos}","lists":{{"all_rooms":{{"count":0}}}},"rooms":{{}},"extensions":{{}}}}"#),
        ))
    }
}

fn kind(path: &str) -> &'static str {
    if path.contains("/login") {
        "login"
    } else if path.contains("/versions") {
        "versions"
    } else if path.contains("/account/whoami") {
        "whoami"
    } else if path.contains("/sync") {
        "sliding"
    } else {
        "other"
    }
}

fn param(path: &str, key: &str) -> Option<u64> {
    let query = path.split_once('?')?.1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
}

/// Reads head and body (body ignored); None when the peer closed first.
async fn read_request(socket: &mut TcpStream) -> Option<(String, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let length = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:")?.trim().parse().ok())
        .unwrap_or(0usize);
    while buf.len() < head_end + length {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let mut line = head.lines().next()?.split(' ');
    Some((line.next()?.to_owned(), line.next()?.to_owned()))
}
