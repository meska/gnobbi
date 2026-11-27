use matrix_desktop::{DemoHistory, HistorySession, HistoryStatus};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistoryPage, HistoryResult};
fn scope(room: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(19),
        room_id: RoomId::new(room),
    }
}
fn event(id: &str) -> HistoryEvent {
    HistoryEvent {
        poll: None,
        redacted: false,
        event_id: EventId::new(id),
        body: "same text".into(),
        outgoing: false,
        timestamp_ms: 1,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        audio: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: None, thread_root: None, reply_to: None,
        status: false, status_row: None,
        reactions: Vec::new(), utd: false,
        attachment_name: None,
        attachment_size: None, media: None,
    }
}
fn page(
    request: &matrix_engine::history::HistoryRequest,
    ids: &[&str],
    next: Option<&str>,
) -> HistoryResult {
    HistoryResult {
        tag: request.tag.clone(),
        page: Ok(HistoryPage {
            tag: request.tag.clone(),
            events: ids.iter().map(|id| event(id)).collect(),
            next: next.map(str::to_owned),
        }),
    }
}

#[test]
fn only_nonempty_start_page_confirms_scan() {
    let mut view = HistorySession::new();
    let first = view.select(Some(scope("!empty:test"))).unwrap();
    view.apply(page(&first, &[], None));
    assert_eq!(view.status() == HistoryStatus::Start, true);
    assert!(!view.confirmed_start());
    let second = view.select(Some(scope("!complete:test"))).unwrap();
    view.apply(page(&second, &["$oldest"], None));
    assert!(view.confirmed_start());
}

#[test]
fn live_activity_refreshes_selected_room_without_replacing_older_cursor() {
    let mut view = HistorySession::new();
    let room = scope("!alpha:test");
    let initial = view.select(Some(room.clone())).unwrap();
    view.apply(page(&initial, &["$old"], Some("older")))
        .unwrap();
    view.activity(&room);
    let refresh = view
        .refresh()
        .expect("selected activity must request latest history");
    assert!(refresh.before.is_none());
    let rows = view
        .apply_live(page(&refresh, &["$old", "$new"], Some("ignored")))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_id.as_str(), "$new");
    assert!(view.refresh().is_none());
    assert_eq!(view.load_older().unwrap().before.as_deref(), Some("older"));
}

#[test]
fn live_refresh_publishes_changed_loaded_source_without_duplicate_unchanged_rows() {
    let mut view = HistorySession::new();
    let room = scope("!links:test");
    let initial = view.select(Some(room.clone())).unwrap();
    view.apply(page(&initial, &["$before", "$link"], Some("older")));
    for body in ["https://new.invalid/è", "same text"] {
        view.activity(&room);
        let refresh = view.refresh().unwrap();
        let mut update = page(&refresh, &["$unloaded", "$before", "$link"], None);
        update.page.as_mut().unwrap().events[2].body = body.into();
        let rows = view.apply_live(update).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "changed loaded link source must be published"
        );
        assert_eq!(rows[0].event_id.as_str(), "$link");
        assert_eq!(rows[0].body, body);
        view.activity(&room);
        let refresh = view.refresh().unwrap();
        let mut unchanged = page(&refresh, &["$before", "$link"], None);
        unchanged.page.as_mut().unwrap().events[1].body = body.into();
        assert!(view.apply_live(unchanged).unwrap().is_empty());
    }
    assert_eq!(view.load_older().unwrap().before.as_deref(), Some("older"));
}

#[test]
fn switching_account_cancels_inflight_live_refresh() {
    let mut view = HistorySession::new();
    let room = scope("!shared:test");
    let initial = view.select(Some(room.clone())).unwrap();
    view.apply(page(&initial, &["$old"], None));
    view.activity(&room);
    let stale = view.refresh().unwrap();
    let mut other = room.clone();
    other.account_id = AccountId::new(20);
    let initial = view.select(Some(other.clone())).unwrap();
    view.apply(page(&initial, &["$other"], None));
    assert!(stale.tag.is_cancelled());
    assert!(view.apply_live(page(&stale, &["$wrong"], None)).is_none());
    view.activity(&room);
    assert!(view.refresh().is_none());
    view.activity(&other);
    assert!(view.refresh().is_some());
}

#[test]
fn live_burst_pages_back_to_overlap_before_publishing_in_chronological_order() {
    let mut view = HistorySession::new();
    let room = scope("!alpha:test");
    let initial = view.select(Some(room.clone())).unwrap();
    view.apply(page(&initial, &["$old"], Some("older")));
    view.activity(&room);
    let newest = view.refresh().unwrap();
    assert!(
        view.apply_live(page(&newest, &["$3", "$4"], Some("gap")))
            .is_none()
    );
    let gap = view
        .refresh()
        .expect("must fill gap, not silently drop a burst");
    assert_eq!(gap.before.as_deref(), Some("gap"));
    let rows = view
        .apply_live(page(&gap, &["$old", "$1", "$2"], Some("stop")))
        .unwrap();
    let ids: Vec<_> = rows.iter().map(|row| row.event_id.as_str()).collect();
    assert_eq!(ids, vec!["$1", "$2", "$3", "$4"]);
    assert!(view.refresh().is_none());
    assert_eq!(view.load_older().unwrap().before.as_deref(), Some("older"));
}

#[test]
fn refresh_overlap_does_not_append_unknown_older_rows() {
    let mut view = HistorySession::new();
    let room = scope("!alpha:test");
    let initial = view.select(Some(room.clone())).unwrap();
    view.apply(page(&initial, &["$old"], Some("older")));
    view.activity(&room);
    let refresh = view.refresh().unwrap();
    let rows = view
        .apply_live(page(&refresh, &["$older-not-loaded", "$old", "$new"], None))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_id.as_str(), "$new");
    let older = view.load_older().unwrap();
    assert_eq!(
        view.apply(page(&older, &["$older-not-loaded"], None))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn live_work_serializes_with_pagination_and_coalesces_activity_during_pending_pages() {
    let mut view = HistorySession::new();
    let room = scope("!alpha:test");
    let initial = view.select(Some(room.clone())).unwrap();
    for _ in 0..100 {
        view.activity(&room);
    }
    assert!(view.refresh().is_none(), "initial page pending");
    view.apply(page(&initial, &["$old"], Some("older")));
    let live = view.refresh().unwrap();
    assert!(
        view.load_older().is_none(),
        "one asynchronous history job at a time"
    );
    for _ in 0..100 {
        view.activity(&room);
    }
    assert!(view.refresh().is_none());
    view.apply_live(page(&live, &["$old", "$new"], None))
        .unwrap();
    let followup = view.refresh().unwrap();
    assert!(view.apply_live(page(&live, &["$stale"], None)).is_none());
    assert_eq!(
        view.apply_live(page(&followup, &["$new", "$newer", "$newer"], None))
            .unwrap()
            .len(),
        1
    );
    assert!(view.refresh().is_none());
    let older = view.load_older().unwrap();
    view.activity(&room);
    assert!(view.refresh().is_none());
    view.apply(page(&older, &["$older"], None));
    assert!(view.refresh().is_some());
}

#[test]
fn refreshed_source_retires_old_links_before_same_body_returns() {
    use matrix_desktop::{LinkConfirmation, MessageLinkRequests};
    let room = scope("!links:test");
    let mut history = DemoHistory::new(room, 0);
    let mut original = event("$link");
    original.body = "https://a.invalid/x".into();
    history.append_server(vec![original]);
    let id = history.events()[0].id.clone();
    let mut links = MessageLinkRequests::default();
    let old = links.targets(&id, &history.events()[0].text).remove(0);
    let mut confirmation = LinkConfirmation::default();
    assert!(links.request(&old, &mut confirmation));
    for body in ["https://b.invalid/y", "https://a.invalid/x"] {
        let mut updated = event("$link");
        updated.body = body.into();
        assert!(history.refresh_server(vec![updated, event("$new")]));
        assert_eq!(history.events().len(), 2);
        assert_eq!(history.events()[0].id, id);
        assert_eq!(history.events()[0].text, body);
        links.retain_current(
            history
                .events()
                .iter()
                .map(|row| (&row.id, row.text.as_str())),
        );
        let current = links.targets(&id, &history.events()[0].text).remove(0);
        assert!(links.request(&current, &mut confirmation));
        assert!(!links.focus(&old));
        assert!(!links.request(&old, &mut confirmation));
        assert_eq!(confirmation.pending().unwrap().as_str(), body);
    }
    assert!(!history.refresh_server(vec![event("$new"), event("$newer")]));
    assert_eq!(history.events().len(), 3);
}

#[test]
fn live_rows_append_without_replacing_loaded_edited_or_redacted_content() {
    let room = scope("!alpha:test");
    let mut history = DemoHistory::new(room, 0);
    let mut edited = event("$edited");
    edited.body = "already edited caffè 🦀".into();
    let mut redacted = event("$redacted");
    redacted.body = "[Message deleted]".into();
    history.prepend_server(vec![edited, redacted]);
    history.append_server(vec![
        event("$edited"),
        event("$redacted"),
        event("$new"),
        event("$new"),
    ]);
    assert_eq!(history.events().len(), 3);
    assert_eq!(history.events()[0].text, "already edited caffè 🦀");
    assert_eq!(history.events()[1].text, "[Message deleted]");
    assert_eq!(
        history.events()[2].id.id,
        matrix_domain::TimelineId::Server(EventId::new("$new"))
    );
}

#[test]
fn failed_live_refresh_retries_latest_without_consuming_older_cursor() {
    let mut view = HistorySession::new();
    let room = scope("!alpha:test");
    let first = view.select(Some(room.clone())).unwrap();
    view.apply(page(&first, &["$old"], Some("older")));
    view.activity(&room);
    let refresh = view.refresh().unwrap();
    assert!(view.is_live(&refresh.tag));
    view.apply_live(HistoryResult {
        tag: refresh.tag.clone(),
        page: Err(matrix_engine::history::HistoryError::Transport),
    });
    assert!(matches!(view.status(), HistoryStatus::Failed));
    assert!(view.refresh().is_none(), "no tight automatic retry loop");
    assert!(
        view.load_older().is_none(),
        "retry failed live work before older pages"
    );
    let retry = view.refresh().unwrap();
    assert!(retry.before.is_none());
    assert!(!view.is_live(&refresh.tag));
    view.apply_live(page(&retry, &["$old", "$new"], None))
        .unwrap();
    assert!(matches!(view.status(), HistoryStatus::More));
    assert_eq!(view.load_older().unwrap().before.as_deref(), Some("older"));
}

#[test]
fn cancelled_live_gap_cannot_publish_after_room_switch_or_removal() {
    for next_room in [Some(scope("!beta:test")), None, Some(scope("!alpha:test"))] {
        let mut view = HistorySession::new();
        let room = scope("!alpha:test");
        let first = view.select(Some(room.clone())).unwrap();
        view.apply(page(&first, &["$old"], None));
        view.activity(&room);
        let live = view.refresh().unwrap();
        view.apply_live(page(&live, &["$buffered"], Some("gap")));
        let gap = view.refresh().unwrap();
        view.select(None);
        if let Some(first) = view.select(next_room) {
            view.apply(page(&first, &["$current"], None));
        }
        assert!(gap.tag.is_cancelled());
        assert!(!view.is_live(&gap.tag));
        assert!(view.apply_live(page(&gap, &["$wrong"], None)).is_none());
        assert!(view.refresh().is_none());
    }
}

#[test]
fn live_cursor_cycle_discards_partial_rows_and_allows_explicit_retry() {
    let mut view = HistorySession::new();
    let room = scope("!alpha:test");
    let initial = view.select(Some(room.clone())).unwrap();
    view.apply(page(&initial, &["$old"], None));
    view.activity(&room);
    let first = view.refresh().unwrap();
    assert!(
        view.apply_live(page(&first, &["$new"], Some("cycle")))
            .is_none()
    );
    let second = view.refresh().unwrap();
    assert!(view.load_older().is_none());
    assert!(
        view.apply_live(page(&second, &["$new"], Some("cycle")))
            .is_none()
    );
    assert!(matches!(view.status(), HistoryStatus::Failed));
    assert!(view.refresh().is_none());
    view.load_older();
    let retry = view.refresh().unwrap();
    assert_eq!(
        view.apply_live(page(&retry, &["$old", "$new"], None))
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(view.status(), HistoryStatus::Start));
}

#[test]
fn mismatched_live_inner_page_cannot_publish_content() {
    let mut view = HistorySession::new();
    let room = scope("!alpha:test");
    let first = view.select(Some(room.clone())).unwrap();
    view.apply(page(&first, &["$old"], None));
    view.activity(&room);
    let refresh = view.refresh().unwrap();
    let mut wrong = page(&first, &["$wrong"], None);
    wrong.tag = refresh.tag;
    assert!(view.apply_live(wrong).is_none());
    assert!(matches!(view.status(), HistoryStatus::Failed));
}

#[test]
fn server_rows_preserve_scoped_identity_body_and_timestamp_without_demo_labels() {
    let scope = scope("!alpha:test");
    let mut history = DemoHistory::new(scope.clone(), 0);
    history.prepend_server(vec![event("$2")]);
    let mut older = event("$1");
    older.timestamp_ms = 3_661_000;
    older.outgoing = true;
    history.prepend_server(vec![older]);
    assert_eq!(history.events().len(), 2);
    let row = &history.events()[0];
    assert_eq!(row.id.scope, scope);
    assert_eq!(
        row.id.id,
        matrix_domain::TimelineId::Server(EventId::new("$1"))
    );
    assert_eq!(row.text, "same text");
    assert_eq!(row.time, matrix_desktop::time_fmt::clock_time(3_661_000));
    assert!(row.outgoing);
    assert!(row.send_state.is_none());
}
#[test]
fn status_row_takes_the_server_timestamp_like_a_bubble() {
    let mut history = DemoHistory::new(scope("!alpha:test"), 0);
    let mut joined = event("$s");
    (joined.status, joined.timestamp_ms) = (true, 4_800_000);
    history.prepend_server(vec![joined]);
    assert_eq!(history.events()[0].time, matrix_desktop::time_fmt::clock_time(4_800_000));
}
#[test]
fn queued_pagination_intent_cannot_load_a_later_page_or_different_room() {
    let mut view = HistorySession::new();
    assert!(view.pagination_tag().is_none());
    let first = view.select(Some(scope("!alpha:test"))).unwrap();
    let intent = view.pagination_tag().unwrap();
    assert!(view.load_older_if_current(&intent).is_none(), "loading");
    view.apply(HistoryResult {
        tag: first.tag.clone(),
        page: Ok(HistoryPage {
            tag: first.tag,
            events: vec![event("$2")],
            next: Some("cursor-1".into()),
        }),
    })
    .unwrap();
    let older = view.load_older_if_current(&intent).unwrap();
    assert_eq!(older.before.as_deref(), Some("cursor-1"));
    assert!(view.load_older_if_current(&intent).is_none(), "duplicate");
    view.apply(HistoryResult {
        tag: older.tag.clone(),
        page: Ok(HistoryPage {
            tag: older.tag,
            events: vec![event("$1")],
            next: Some("cursor-2".into()),
        }),
    })
    .unwrap();
    assert!(
        view.load_older_if_current(&intent).is_none(),
        "old page even after completion"
    );
    let current = view.pagination_tag().unwrap();
    view.select(Some(scope("!beta:test")));
    assert!(view.load_older_if_current(&current).is_none(), "other room");
    view.select(None);
    assert!(view.pagination_tag().is_none());
}

#[test]
fn pagination_deduplicates_server_ids_without_matching_text() {
    let mut view = HistorySession::new();
    let first = view.select(Some(scope("!alpha:test"))).unwrap();
    assert!(view.load_older().is_none(), "one in flight");
    let rows = view
        .apply(HistoryResult {
            tag: first.tag.clone(),
            page: Ok(HistoryPage {
                tag: first.tag,
                events: vec![event("$2"), event("$3")],
                next: Some("opaque-not-event-id".into()),
            }),
        })
        .unwrap();
    assert_eq!(rows.len(), 2);
    let older = view.load_older().unwrap();
    assert_eq!(older.before.as_deref(), Some("opaque-not-event-id"));
    let rows = view
        .apply(HistoryResult {
            tag: older.tag.clone(),
            page: Ok(HistoryPage {
                tag: older.tag,
                events: vec![event("$1"), event("$2"), event("$1")],
                next: None,
            }),
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_id.as_str(), "$1");
    assert!(view.load_older().is_none(), "history exhausted");
}

#[test]
fn account_switch_rejects_stale_same_room_pages_without_sharing_seen_ids() {
    let mut view = HistorySession::new();
    let alice_scope = scope("!shared:test");
    let mut bob_scope = alice_scope.clone();
    bob_scope.account_id = AccountId::new(28);
    let old = view.select(Some(alice_scope.clone())).unwrap();
    let bob = view.select(Some(bob_scope)).unwrap();
    assert!(old.tag.is_cancelled());
    assert!(
        view.apply(HistoryResult {
            tag: old.tag.clone(),
            page: Ok(HistoryPage {
                tag: old.tag,
                events: vec![event("$shared")],
                next: Some("alice-cursor".into())
            }),
        })
        .is_none()
    );
    let rows = view
        .apply(HistoryResult {
            tag: bob.tag.clone(),
            page: Ok(HistoryPage {
                tag: bob.tag.clone(),
                events: vec![event("$shared")],
                next: Some("bob-cursor".into()),
            }),
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        view.load_older().unwrap().before.as_deref(),
        Some("bob-cursor")
    );
    let alice = view.select(Some(alice_scope)).unwrap();
    let rows = view
        .apply(HistoryResult {
            tag: alice.tag.clone(),
            page: Ok(HistoryPage {
                tag: alice.tag,
                events: vec![event("$shared")],
                next: None,
            }),
        })
        .unwrap();
    assert_eq!(rows.len(), 1, "seen event IDs cannot leak across accounts");
    view.select(None);
    assert!(
        view.apply(HistoryResult {
            tag: bob.tag.clone(),
            page: Ok(HistoryPage {
                tag: bob.tag,
                events: vec![event("$shared")],
                next: None
            }),
        })
        .is_none()
    );
}

#[test]
fn room_switch_rejects_already_queued_old_page() {
    let mut view = HistorySession::new();
    let old = view.select(Some(scope("!alpha:test"))).unwrap();
    let new = view.select(Some(scope("!beta:test"))).unwrap();
    assert!(old.tag.is_cancelled());
    assert!(
        view.apply(HistoryResult {
            tag: old.tag.clone(),
            page: Ok(HistoryPage {
                tag: old.tag,
                events: vec![event("$old")],
                next: None
            })
        })
        .is_none()
    );
    let accepted = view
        .apply(HistoryResult {
            tag: new.tag.clone(),
            page: Ok(HistoryPage {
                tag: new.tag,
                events: vec![event("$new")],
                next: Some("opaque".into()),
            }),
        })
        .unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].event_id.as_str(), "$new");
}

#[test]
fn failed_page_retries_same_cursor_without_losing_seen_ids() {
    let mut view = HistorySession::new();
    assert!(matches!(view.status(), HistoryStatus::Empty));
    let first = view.select(Some(scope("!alpha:test"))).unwrap();
    assert!(view.select(Some(scope("!alpha:test"))).is_none());
    assert!(!first.tag.is_cancelled());
    assert!(matches!(view.status(), HistoryStatus::Loading));
    view.apply(HistoryResult {
        tag: first.tag.clone(),
        page: Ok(HistoryPage {
            tag: first.tag,
            events: vec![event("$2")],
            next: Some("cursor".into()),
        }),
    })
    .unwrap();
    let older = view.load_older().unwrap();
    assert!(
        view.apply(HistoryResult {
            tag: older.tag,
            page: Err(matrix_engine::history::HistoryError::Transport),
        })
        .is_none()
    );
    assert!(matches!(view.status(), HistoryStatus::Failed));
    let retry = view.load_older().unwrap();
    assert_eq!(retry.before.as_deref(), Some("cursor"));
    let rows = view
        .apply(HistoryResult {
            tag: retry.tag.clone(),
            page: Ok(HistoryPage {
                tag: retry.tag,
                events: vec![event("$1"), event("$2")],
                next: None,
            }),
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_id.as_str(), "$1");
    assert!(matches!(view.status(), HistoryStatus::Start));
}

#[test]
fn removing_selected_room_cancels_pending_history_and_resets_lifecycle() {
    let mut view = HistorySession::new();
    let old = view.select(Some(scope("!alpha:test"))).unwrap();
    assert!(view.select(None).is_none());
    assert!(old.tag.is_cancelled());
    assert!(matches!(view.status(), HistoryStatus::Empty));
    assert!(view.load_older().is_none());
    assert!(
        view.apply(HistoryResult {
            tag: old.tag.clone(),
            page: Ok(HistoryPage {
                tag: old.tag,
                events: vec![event("$old")],
                next: None,
            }),
        })
        .is_none()
    );
    let fresh = view.select(Some(scope("!alpha:test"))).unwrap();
    let rows = view
        .apply(HistoryResult {
            tag: fresh.tag.clone(),
            page: Ok(HistoryPage {
                tag: fresh.tag,
                events: vec![event("$old")],
                next: None,
            }),
        })
        .unwrap();
    assert_eq!(rows.len(), 1, "a new lifecycle must not inherit seen IDs");
}

#[test]
fn scrolling_near_the_top_starts_one_older_page_and_none_at_room_start() {
    let mut view = HistorySession::new();
    let room = scope("!auto:test");
    let initial = view.select(Some(room)).unwrap();
    view.apply(page(&initial, &["$a", "$b"], Some("older")))
        .unwrap();
    assert!(
        view.load_older_near_top(HistorySession::AUTO_LOAD_ROWS + 1)
            .is_none()
    );
    let request = view
        .load_older_near_top(HistorySession::AUTO_LOAD_ROWS)
        .expect("near the top must request the previous page");
    assert_eq!(request.before.as_deref(), Some("older"));
    assert!(
        view.load_older_near_top(0).is_none(),
        "one request at a time"
    );
    view.apply(page(&request, &["$c"], None)).unwrap();
    assert!(
        view.load_older_near_top(0).is_none(),
        "nothing at room start"
    );
}

#[test]
fn failed_older_page_retries_only_after_the_reader_scrolls() {
    let mut view = HistorySession::new();
    let initial = view.select(Some(scope("!retry:test"))).unwrap();
    view.apply(page(&initial, &["$a"], Some("older"))).unwrap();
    let request = view.load_older_near_top(3).unwrap();
    assert!(
        view.apply(HistoryResult {
            tag: request.tag,
            page: Err(matrix_engine::history::HistoryError::Transport),
        })
        .is_none()
    );
    assert!(matches!(view.status(), HistoryStatus::Failed));
    for _ in 0..3 {
        assert!(
            view.load_older_near_top(3).is_none(),
            "no retry while the view stands still"
        );
    }
    let retry = view.load_older_near_top(2).expect("scrolling retries once");
    assert_eq!(retry.before.as_deref(), Some("older"));
    assert!(
        view.load_older_near_top(1).is_none(),
        "one request at a time"
    );
    view.apply(page(&retry, &["$b"], None)).unwrap();
    assert!(matches!(view.status(), HistoryStatus::Start));
}

#[test]
fn selecting_a_room_asks_to_open_it_and_the_saved_page_then_the_gap_both_land() {
    let mut view = HistorySession::new();
    let room = scope("!seen:test");
    let open = view.select(Some(room.clone())).unwrap();
    assert!(open.open, "selection opens through the saved messages");
    let saved = view.apply(page(&open, &["$a", "$b"], None)).unwrap();
    assert_eq!(saved.len(), 2);
    let gap = view.apply(page(&open, &["$a", "$b", "$c"], None)).unwrap();
    assert_eq!(gap.len(), 1);
    assert_eq!(gap[0].event_id.as_str(), "$c");
    view.activity(&room);
    assert!(
        !view.refresh().unwrap().open,
        "live refresh stays a plain page"
    );
}

#[test]
fn a_formatted_refresh_replaces_the_row_styles_even_with_the_same_text() {
    use matrix_engine::rich_text::{Style, StyleSpan};
    let mut history = DemoHistory::new(scope("!fmt:test"), 0);
    history.append_server(vec![event("$m")]);
    assert!(history.events()[0].styles.is_empty());
    let mut formatted = event("$m");
    formatted.styles = vec![StyleSpan {
        range: 0..4,
        style: Style::Bold,
    }];
    assert!(history.refresh_server(vec![formatted]));
    assert_eq!(history.events()[0].styles.len(), 1);
    assert!(history.refresh_server(vec![event("$m")]));
    assert!(history.events()[0].styles.is_empty());
}

#[test]
fn live_refresh_hands_over_the_thread_root_when_a_reply_arrives() {
    let mut view = HistorySession::new();
    let room = scope("!thread:test");
    let initial = view.select(Some(room.clone())).unwrap();
    let with_replies = |n| {
        let mut root = event("$root");
        root.thread = Some(matrix_engine::history::ThreadInfo { num_replies: n, latest_sender: None, unread: 0 });
        root
    };
    let mut first = page(&initial, &[], None);
    first.page.as_mut().unwrap().events = vec![with_replies(1)];
    view.apply(first);
    view.activity(&room);
    let refresh = view.refresh().unwrap();
    let mut update = page(&refresh, &["$reply"], None);
    update.page.as_mut().unwrap().events.insert(0, with_replies(2));
    let rows = view.apply_live(update).unwrap();
    let ids: Vec<_> = rows.iter().map(|r| (r.event_id.as_str(), r.thread.as_ref().map(|t| t.num_replies))).collect();
    assert_eq!(ids, [("$root", Some(2)), ("$reply", None)], "root comes again with the new count");
}

#[test]
fn a_room_with_threads_is_read_once_more_after_new_rows() {
    let mut view = HistorySession::new();
    let room = scope("!settle:test");
    let initial = view.select(Some(room.clone())).unwrap();
    let mut root = event("$root");
    root.thread = Some(matrix_engine::history::ThreadInfo { num_replies: 1, latest_sender: None, unread: 0 });
    let mut first = page(&initial, &[], None);
    first.page.as_mut().unwrap().events = vec![root];
    view.apply(first);
    view.activity(&room);
    let refresh = view.refresh().unwrap();
    let mut live = page(&refresh, &["$root", "$reply"], None);
    live.page.as_mut().unwrap().events[0].thread = Some(matrix_engine::history::ThreadInfo { num_replies: 1, latest_sender: None, unread: 0 });
    view.apply_live(live).unwrap();
    assert!(view.refresh().is_none(), "not before the delay");
    std::thread::sleep(matrix_desktop::SETTLE);
    assert!(view.refresh().is_some(), "the summary may have moved after the reply row");
    // A room without threads is read once more after activity (the Timeline may lag), then rests.
    let mut plain = HistorySession::new();
    let initial = plain.select(Some(room.clone())).unwrap();
    plain.apply(page(&initial, &["$a"], None));
    plain.activity(&room);
    let refresh = plain.refresh().unwrap();
    plain.apply_live(page(&refresh, &["$a", "$b"], None)).unwrap();
    std::thread::sleep(matrix_desktop::SETTLE);
    let refresh = plain.refresh().expect("a redaction the Timeline applied late must be read");
    assert!(plain.apply_live(page(&refresh, &["$a", "$b"], None)).unwrap().is_empty());
    std::thread::sleep(matrix_desktop::SETTLE);
    assert!(plain.refresh().is_none(), "the settle read does not re-arm itself");
}

#[test]
fn live_refresh_publishes_a_poll_whose_votes_or_end_changed_with_the_same_body() {
    let poll = |votes, ended| matrix_domain::Poll {
        question: "same text".into(),
        answers: vec![matrix_domain::PollAnswer { id: "a1".into(), text: "Si".into(), votes, own: votes > 0 }],
        max_selections: 1,
        multiple: false,
        disclosed: true,
        ended,
        voters: votes,
        can_end: !ended,
    };
    let mut view = HistorySession::new();
    let room = scope("!poll:test");
    let initial = view.select(Some(room.clone())).unwrap();
    let mut first = page(&initial, &["$poll"], None);
    first.page.as_mut().unwrap().events[0].poll = Some(poll(0, false));
    view.apply(first);
    for (votes, ended) in [(1, false), (1, true)] {
        view.activity(&room);
        let refresh = view.refresh().unwrap();
        let mut update = page(&refresh, &["$poll"], None);
        update.page.as_mut().unwrap().events[0].poll = Some(poll(votes, ended));
        let rows = view.apply_live(update).unwrap();
        assert_eq!(rows.len(), 1, "a poll with new votes or closed must be published");
        assert_eq!(rows[0].poll, Some(poll(votes, ended)));
    }
}

#[test]
fn a_loading_quote_is_read_again_and_republished_once_resolved() {
    use matrix_engine::history::{ReplyRef, ReplyState};
    let quote = |state| Some(ReplyRef { event_id: EventId::new("$orig"), author: (state == ReplyState::Resolved).then(|| "Omar".into()), preview: None, placeholder: false, state });
    let mut view = HistorySession::new();
    let room = scope("!quote:test");
    let initial = view.select(Some(room.clone())).unwrap();
    let mut first = page(&initial, &["$reply"], None);
    first.page.as_mut().unwrap().events[0].reply_to = quote(ReplyState::Loading);
    view.apply(first).unwrap();
    assert!(view.refresh().is_none(), "no activity yet: the settle delay has not passed");
    std::thread::sleep(matrix_desktop::SETTLE);
    let refresh = view.refresh().expect("a loading quote is read again by itself");
    // Still Loading and identical to the seen row: nothing is published, but the read must re-arm.
    let mut same = page(&refresh, &["$reply"], None);
    same.page.as_mut().unwrap().events[0].reply_to = quote(ReplyState::Loading);
    assert!(view.apply_live(same).unwrap().is_empty());
    std::thread::sleep(matrix_desktop::SETTLE);
    let refresh = view.refresh().expect("an unchanged loading quote keeps being read");
    let mut done = page(&refresh, &["$reply"], None);
    done.page.as_mut().unwrap().events[0].reply_to = quote(ReplyState::Resolved);
    let rows = view.apply_live(done).unwrap();
    assert_eq!(rows.len(), 1, "the resolved quote replaces the loading row");
    assert_eq!(rows[0].reply_to.as_ref().unwrap().state, ReplyState::Resolved);
}

#[test]
fn live_refresh_publishes_a_row_whose_reactions_changed_with_the_same_body() {
    let like = |count| vec![matrix_engine::history::Reaction { key: "👍".into(), count, senders: Vec::new(), mine: false, pending: false }];
    let mut view = HistorySession::new();
    let room = scope("!react:test");
    let initial = view.select(Some(room.clone())).unwrap();
    view.apply(page(&initial, &["$m"], None));
    for count in [1, 2] {
        view.activity(&room);
        let refresh = view.refresh().unwrap();
        let mut update = page(&refresh, &["$m"], None);
        update.page.as_mut().unwrap().events[0].reactions = like(count);
        let rows = view.apply_live(update).unwrap();
        assert_eq!(rows.len(), 1, "a new reaction must be published");
        assert_eq!(rows[0].reactions, like(count));
    }
}
