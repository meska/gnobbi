//! Caption under an attachment: the row text IS the caption (empty when there is none), the
//! kind/label travel in `media`, the file name stays attachment data, the accessible name is
//! «{label}, {caption}» and a caption arriving later (edit, decryption) replaces the old row.
use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender, Media, MediaKind};

fn scope() -> RoomScope {
    RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!cap:test") }
}

fn row(id: &str, kind: Option<MediaKind>, label: &str, caption: &str) -> HistoryEvent {
    HistoryEvent {
        poll: None,
        event_id: EventId::new(id),
        body: caption.into(),
        outgoing: false,
        redacted: false,
        utd: false,
        timestamp_ms: 1,
        sender: HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: None,
        thread_root: None,
        status: false,
        attachment_name: kind.map(|_| "dati.png".into()),
        attachment_size: kind.map(|_| 10),
        media: kind.map(|kind| Media { kind, label: label.into() }),
    }
}

#[test]
fn an_edit_or_a_late_decryption_replaces_the_caption_and_the_media() {
    let mut history = DemoHistory::new(scope(), 0);
    history.append_server(vec![row("$a:test", None, "", "Cifrato · chiavi non disponibili"), row("$b:test", Some(MediaKind::Image), "Immagine", "Prima")]);
    // $a is decrypted into a captioned image; $b gets a new caption.
    assert!(history.refresh_server(vec![row("$a:test", Some(MediaKind::Image), "Immagine", "Ecco 😀"), row("$b:test", Some(MediaKind::Image), "Immagine", "Dopo")]));
    let rows = history.events();
    assert_eq!((rows[0].text.as_str(), rows[0].media.as_ref().map(|m| m.kind)), ("Ecco 😀", Some(MediaKind::Image)));
    assert_eq!(rows[1].text, "Dopo");
    // The caption is removed: no text left, still an image, and the file stays saveable.
    assert!(history.refresh_server(vec![row("$b:test", Some(MediaKind::Image), "Immagine", "")]));
    assert_eq!(history.events()[1].text, "");
    assert!(history.events()[1].media.is_some() && history.events()[1].attachment_name.is_some());
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
#[test]
fn the_accessible_name_is_label_then_caption_with_no_help_text() {
    use matrix_desktop::accessibility::HistoryNodes;
    let mut history = DemoHistory::new(scope(), 0);
    history.append_server(vec![
        row("$img:test", Some(MediaKind::Image), "Immagine", "Ecco i dati fiscali"),
        row("$bare:test", Some(MediaKind::Image), "Immagine", ""),
        row("$file:test", Some(MediaKind::File), "File · a.pdf · 2.0 MB", "Il contratto"),
        row("$vid:test", Some(MediaKind::Video), "Video · 0:12", ""),
        row("$txt:test", None, "", "ciao"),
    ]);
    let projected = HistoryNodes::default().project(&history, 0, true);
    let names: Vec<_> = projected.messages.iter().map(|m| (m.name.as_str(), m.text.as_str())).collect();
    assert_eq!(
        names,
        [
            ("Immagine, Ecco i dati fiscali", "Ecco i dati fiscali"),
            ("Immagine", ""),
            ("File · a.pdf · 2.0 MB, Il contratto", "Il contratto"),
            ("Video · 0:12", ""),
            ("ciao", "ciao"),
        ]
    );
    assert!(projected.messages.iter().all(|m| !m.name.contains("dati.png")), "the file name is not text");
    // The ↓ stays on every attachment row.
    assert_eq!(projected.messages.iter().filter(|m| m.save.is_some()).count(), 4);
}
