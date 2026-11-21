//! Runs one extension fetch and prints only kinds, flags and lengths (never text). usage: probe <data_dir> <tag> <pass> <room> <event>
//! EXPECT_BODY / EXPECT_ROOM / EXPECT_SENDER (env): compared in-process, printed as true/false.
use gnobbi_nse::{Outcome, run};
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let t = std::time::Instant::now();
    let r = run(Path::new(&a[1]), &a[2], &a[3], &a[4], &a[5]);
    let ms = t.elapsed().as_millis();
    let is = |name: &str, v: &str| std::env::var(name).map(|e| e == v).unwrap_or(false);
    match r {
        Ok(Outcome::Message { sender, body, room }) => println!(
            "message body_ok={} room_ok={} sender_ok={} sender_len={} ms={ms}",
            is("EXPECT_BODY", &body),
            is("EXPECT_ROOM", room.as_deref().unwrap_or("")),
            is("EXPECT_SENDER", &sender),
            sender.len()
        ),
        Ok(o) => println!(
            "{} ms={ms}",
            format!("{o:?}").split(['{', ' ']).next().unwrap_or("")
        ),
        Err(e) => println!("fail {e:?} ms={ms}"),
    }
}
