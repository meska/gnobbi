//! «Scarica» (↓) in the accessibility tree: one button per attachment, received or sent, image or file,
//! always named exactly "Scarica" and clickable; none on text, on a deleted row or on a row still sending.
//! Painted size (≥32×32) and Tab/Shift-Tab are checked on a real window by `painted_save_icons_*` in
//! src/timeline_view.rs; the engine side (hash of the saved file) by matrix-engine/tests/save_fresh_session.rs.
#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::{
    DemoHistory,
    accessibility::{HistoryNodes, NavigationAction, NavigationSnapshot},
};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn snapshot(history: matrix_desktop::accessibility::HistorySnapshot) -> NavigationSnapshot {
    NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        disconnected: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: Some(history),
        list_focused: false,
        composer: None,
    }
}

fn scope() -> RoomScope {
    RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!save:test") }
}

fn event(id: &str, outgoing: bool, name: Option<&str>, image: bool, redacted: bool) -> HistoryEvent {
    HistoryEvent {
        poll: None,
        event_id: EventId::new(id),
        body: if image { String::new() } else { name.unwrap_or("solo testo").into() },
        outgoing,
        redacted,
        timestamp_ms: 1,
        sender: HistorySender::new(if outgoing { "@me:test" } else { "@peer:test" }, None),
        video: None,
        audio: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: None,
        thread_root: None,
        status: false,
        utd: false,
        attachment_name: name.map(Into::into),
        attachment_size: name.map(|_| 10), media: image.then(|| matrix_engine::history::Media { kind: matrix_engine::history::MediaKind::Image, label: "Immagine".into() }),
    }
}

#[test]
fn every_attachment_has_the_same_scarica_button_text_deleted_and_sending_rows_have_none() {
    let mut history = DemoHistory::new(scope(), 0);
    history.submit_text("ancora in invio".into()); // still sending: Pending id
    history.prepend_server(vec![
        event("$img-in:test", false, Some("foto.png"), true, false),
        event("$file-in:test", false, Some("rel.pdf"), false, false),
        event("$img-out:test", true, Some("mia.png"), true, false),
        event("$file-out:test", true, Some("mio.pdf"), false, false),
        event("$text:test", false, None, false, false),
        event("$gone:test", false, Some("cancellato.pdf"), false, true),
    ]);
    let rows: Vec<_> = history.events().iter().map(|e| e.id.clone()).collect();
    let mut projected = HistoryNodes::default().project(&history, 0, true);
    let with_icon: Vec<_> = projected.messages.iter().filter(|m| m.save.is_some()).map(|m| m.event.clone()).collect();
    assert_eq!(with_icon.len(), 4, "received+sent × image+file, nothing else: {with_icon:?}");
    for (message, row) in projected.messages.iter_mut().zip(&rows) {
        if message.save.is_some() {
            // The painted size, as the window reports it (32 px icon).
            message.save_bounds = Some(accesskit::Rect::new(100., 200., 132., 232.));
            assert_eq!(message.event, *row);
        }
    }
    let current = snapshot(projected);
    let tree = current.tree();
    let buttons: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.label() == Some("Scarica")).collect();
    assert_eq!(buttons.len(), 4);
    // The accessible name is one word for files and images, received and sent; no other node borrows it.
    assert!(tree.nodes.iter().all(|(_, n)| n.label().is_none_or(|l| !l.starts_with("Scarica") || l == "Scarica")));
    for (id, node) in buttons {
        assert_eq!(node.role(), Role::Button);
        assert!(!node.is_disabled(), "an offered ↓ is live");
        let bounds = node.bounds().unwrap();
        assert!(bounds.width() >= 32. && bounds.height() >= 32., "{bounds:?}");
        let click = ActionRequest { target_tree: TreeId::ROOT, target_node: *id, action: Action::Click, data: None };
        let target = match current.action(&click) {
            Some(NavigationAction::SaveAttachment(target)) => target,
            _ => panic!("click on ↓ is not SaveAttachment"),
        };
        assert!(with_icon.contains(&target), "the click saves the row it belongs to");
    }
}

#[test]
fn a_row_still_sending_exposes_no_scarica_node_at_all() {
    let mut history = DemoHistory::new(scope(), 0);
    history.submit_text("foto.png".into());
    let projected = HistoryNodes::default().project(&history, 0, true);
    assert!(projected.messages.iter().all(|m| m.save.is_none()));
    let tree = snapshot(projected).tree();
    assert!(tree.nodes.iter().all(|(_, n)| n.label() != Some("Scarica")), "absent, not a disabled button with text");
}
