//! Frame-time harness for the room info member list (synthetic members, no server).
//! Run in release: `cargo test --release -p matrix-desktop --bin gnobbi members_bench -- --ignored --nocapture`
use eframe::egui;
use matrix_domain::role::MemberRole;
use matrix_engine::operations::{Member, MemberPermissions};
use std::time::{Duration, Instant};

pub fn synthetic(n: usize, with_avatar: bool) -> Vec<Member> {
    (0..n)
        .map(|i| Member {
            user_id: format!("@user{i}:example.org"),
            // Every 50th name repeats, as real rooms do: those rows take the two-line path.
            name: if i % 50 == 0 { "Marco".into() } else { format!("Utente numero {i}") },
            avatar: with_avatar.then(|| format!("mxc://example.org/{i}")),
            role: match i { 0..=2 => MemberRole::Admin, 3..=9 => MemberRole::Moderator, _ => MemberRole::Member },
        })
        .collect()
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

/// Median CPU time of one full frame (layout + tessellation) of the panel, after warm-up; also the shape count.
fn frame(members: &[Member], admin: Option<&crate::members::Admin>, page: bool) -> (Duration, usize) {
    let scope = matrix_domain::RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!r:example.org") };
    let mut list = crate::members::MemberList::new(scope, members.to_vec(), &Default::default(), (0, 0, 0));
    let mut query = String::new();
    let ctx = egui::Context::default();
    let avatars = Default::default();
    let (mut times, mut shapes) = (Vec::new(), 0);
    for step in 0..14 {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 800.))), ..Default::default() };
        let start = Instant::now();
        let out = ctx.run_ui(input, |ui| {
            if page {
                crate::members::page(ui, &mut list, &mut query, None, admin, "@me:example.org", &avatars, &matrix_desktop::LIGHT);
            } else {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    crate::members::preview(ui, &list, admin, "@me:example.org", &mut false, &avatars, &matrix_desktop::LIGHT);
                });
            }
        });
        shapes = out.shapes.len();
        let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
        if step >= 4 {
            times.push(start.elapsed());
        }
    }
    (median(times), shapes)
}

#[test]
#[ignore = "measurement, run by hand in release"]
fn frame_time_by_member_count() {
    let mut admin = crate::members::Admin::new(matrix_domain::RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!r:example.org") });
    admin.perms = MemberPermissions { can_invite: true, can_kick: true, can_ban: true };
    for n in [100, 1000, 8000] {
        let members = synthetic(n, true);
        let (plain, shapes) = frame(&members, None, true);
        let (moderator, _) = frame(&members, Some(&admin), true);
        let (preview, _) = frame(&members, Some(&admin), false);
        let t = Instant::now();
        let dup = crate::rows::duplicate_names(members.iter().map(|m| m.name.as_str()));
        let dup_time = t.elapsed();
        let mut sorted = members.clone();
        let t = Instant::now();
        sorted.sort_by(|a, b| matrix_domain::role::cmp_by_role_then_name((a.role, &a.name), (b.role, &b.name)));
        let sort_time = t.elapsed();
        let t = Instant::now();
        let copy = members.clone();
        let clone_time = t.elapsed();
        println!("N={n:5} frame(member)={plain:>9.2?} frame(moderator)={moderator:>9.2?} preview(moderator)={preview:>9.2?} shapes={shapes} duplicate_names={dup_time:.2?} ({}) sort_once={sort_time:.2?} clone_once={clone_time:.2?} ({})", dup.len(), copy.len());
    }
}
