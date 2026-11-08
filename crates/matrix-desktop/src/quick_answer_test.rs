//! Quick search: a join/DM started from the overlay keeps it open until the engine answers.
//! Err: overlay stays, one error line, field enabled again, no recent. Ok: overlay closes, recent saved.
//!   the ct script 'the gui-test-x11 script "" quick_search_join_dm_answer'
use crate::*;
use gpui::{AsyncWindowContext, BackgroundExecutor, WeakEntity};
use matrix_desktop::quick_search::Recent;
use matrix_engine::operations::{OperationCommand, OperationError};

pub(crate) async fn scenario(this: WeakEntity<Shell>, cx: &mut AsyncWindowContext, executor: BackgroundExecutor) {
    let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
    wait(300).await;
    let user = || Recent { id: "@ada:fixture.test".into(), name: "Ada".into(), avatar: None };
    this.update_in(cx, |this, window, cx| {
        this.pending_operation_command = None;
        this.open_quick_search(window, cx);
        this.quick_pick_user(user(), window, cx);
        assert!(matches!(&this.pending_operation_command, Some(OperationCommand::StartDm { .. })));
        assert!(this.quick_busy_for_test(), "overlay stays open and busy while the engine works");
        this.room_started(Err(OperationError::Unavailable), cx);
        assert_eq!(this.quick_error_for_test(), Some("Utente non trovato"));
        assert!(!this.quick_busy_for_test() && this.quick_field_enabled_for_test(cx), "field enabled again");
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        assert!(has_error_label(this, window, cx, "Utente non trovato"), "error exposed to screen readers");
        assert!(this.recents.get(this.active_scope.account_id.as_u64()).searched().is_empty(), "no recent on Err");
        this.pending_operation_command = None;
        this.quick_pick_user(user(), window, cx);
        assert_eq!(this.quick_error_for_test(), None, "a new attempt clears the error");
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        assert!(!has_error_label(this, window, cx, "Utente non trovato"), "label gone with the error");
        let scope = RoomScope { account_id: this.active_scope.account_id, room_id: matrix_domain::RoomId::new("!dm:fixture.test") };
        this.room_started(Ok(scope), cx);
        assert!(this.quick_error_for_test().is_none() && !this.quick_open_for_test(), "Ok closes the overlay");
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        assert!(this.semantic_quick_search(window, cx).is_none(), "no snapshot after Ok");
        assert_eq!(this.recents.get(this.active_scope.account_id.as_u64()).searched().len(), 1, "recent saved on Ok");
    })
    .unwrap();
    this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
fn has_error_label(this: &Shell, window: &Window, cx: &gpui::App, text: &str) -> bool {
    let snap = this.semantic_quick_search(window, cx).expect("overlay open");
    snap.items.iter().any(|i| i.role == accesskit::Role::Label && i.label == text)
}
