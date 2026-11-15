//! «Stanze pubbliche» rows while a join runs: «Ingresso…» at once, no second command, other rows and the popup stay usable,
//! every state on its row, late success with the popup closed = notice without navigation.
//!   the ct script 'the gui-test-wayland script "linux-a11y" quick_join_row'   (P109_DARK=1: dark; screenshots in $SHOT_DIR)
use crate::{quick_search_view::Zone, *};
use gpui::{AsyncWindowContext, BackgroundExecutor, WeakEntity};
use matrix_desktop::quick_search::Category;
use matrix_engine::{
    join::JoinPhase,
    operations::{DirectoryRoom, OperationCommand, OperationError},
    search::{More, SearchState},
};

fn room(n: usize, name: &str) -> DirectoryRoom {
    DirectoryRoom {
        room_id: format!("!r{n}:matrix.org"),
        alias: Some(format!("#{}:matrix.org", name.to_lowercase())),
        name: name.into(),
        topic: None,
        members: 1000 * n as u64,
        avatar_url: None,
        world_readable: false,
        guest_can_join: false,
        join_rule: "public".into(),
        origin: "matrix.org".into(),
    }
}

pub(crate) async fn scenario(this: WeakEntity<Shell>, cx: &mut AsyncWindowContext, executor: BackgroundExecutor) {
    let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
    let tag = format!(
        "join-row-{}-{}",
        if std::env::var("P109_DARK").as_deref() == Ok("1") { "dark" } else { "light" },
        std::env::var("GUI_SCALE").unwrap_or_else(|_| "1".into())
    );
    wait(400).await;
    this.update_in(cx, |this, window, cx| {
        this.pending_operation_command = None;
        this.open_quick_search(window, cx);
        let q = this.quick.as_mut().unwrap();
        q.chip = Some(Category::Rooms);
        q.rooms.set_suggesting();
        let hits = ["Matrix HQ", "Rust", "Grande stanza federata", "Privata", "Libera"].iter().enumerate().map(|(i, n)| room(i + 1, n)).collect();
        assert!(q.rooms.set_state(SearchState::Results { term: String::new(), hits, more: More::None }));
        let pick = |this: &mut Shell, i, window: &mut Window, cx: &mut Context<Shell>| {
            this.pending_operation_command = None;
            this.quick_pick(Zone::Rooms, i, window, cx);
        };
        // Row 0: one click = one JoinRoom, the row is pending at once, the popup and its field stay usable.
        pick(this, 0, window, cx);
        assert!(matches!(&this.pending_operation_command, Some(OperationCommand::JoinRoom { target, .. }) if target == "!r1:matrix.org"));
        assert!(!this.quick_busy_for_test() && this.quick_field_enabled_for_test(cx), "popup stays usable");
        // Second click on the same row: nothing is sent.
        pick(this, 0, window, cx);
        assert!(this.pending_operation_command.is_none(), "no second JoinRoom");
        // Other rows work and reach every state.
        for i in 1..4 {
            pick(this, i, window, cx);
            assert!(this.pending_operation_command.is_some(), "row {i} usable");
        }
        this.join_progress("!r2:matrix.org", JoinPhase::Slow, cx);
        this.join_progress("!r3:matrix.org", JoinPhase::Refused(OperationError::Forbidden), cx);
        this.join_progress("!r4:matrix.org", JoinPhase::Unconfirmed, cx);
        assert!(this.quick_open_for_test(), "popup stays open");
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let snap = this.semantic_quick_search(window, cx).unwrap();
            let has = |l: &str| snap.items.iter().any(|i| i.role == accesskit::Role::Label && i.label == l);
            assert!(has("Matrix HQ, ingresso"), "pending exposed: {:?}", snap.items.iter().map(|i| &i.label).collect::<Vec<_>>());
            assert!(has("Rust, ancora in corso"));
            assert!(has("Privata, non confermato"));
        }
        // «Riprova» = the same click sends again.
        pick(this, 3, window, cx);
        assert!(this.pending_operation_command.is_some(), "retry sends a new JoinRoom");
        this.join_progress("!r4:matrix.org", JoinPhase::Unconfirmed, cx);
        cx.notify();
    })
    .unwrap();
    wait(700).await;
    this.update_in(cx, |_, window, _| {
        crate::ink_test::note(&format!("{tag}: scale {}", window.scale_factor()));
        crate::ink_test::grim(&tag, window);
    })
    .unwrap();
    this.update_in(cx, |this, window, cx| {
        // Closed popup: the late success only sets the notice, no navigation, no popup.
        this.close_quick_search(window, cx);
        let scope = RoomScope { account_id: this.active_scope.account_id, room_id: matrix_domain::RoomId::new("!r1:matrix.org") };
        let before = this.active_scope.clone();
        this.join_progress("!r1:matrix.org", JoinPhase::Joined(scope), cx);
        assert_eq!(this.joins.notice().map(|n| n.name.as_str()), Some("Matrix HQ"));
        assert!(!this.quick_open_for_test() && this.active_scope == before, "no popup, no navigation");
    })
    .unwrap();
    wait(400).await;
    this.update_in(cx, |_, window, _| {
        crate::ink_test::grim(&format!("{tag}-notice"), window);
    })
    .unwrap();
    this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
}
