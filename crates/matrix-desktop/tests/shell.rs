#[test]
fn shell_fixture_is_explicitly_offline_and_deterministic() {
    use matrix_desktop::{DEMO_NOTICE, MESSAGES, ROOMS};
    assert_eq!(DEMO_NOTICE, "DEMO OFFLINE · nessun account connesso");
    assert_eq!(ROOMS.len(), 6);
    assert_eq!(ROOMS[0].name, "Laboratorio UI");
    assert_eq!(ROOMS[0].initials, "LU");
    assert_eq!(MESSAGES.len(), 5);
    assert_eq!(
        MESSAGES[0].text,
        "Questa è la nuova shell desktop. Tutti i contenuti sono fixture locali."
    );
    assert!(!MESSAGES[0].outgoing);
    assert!(MESSAGES[1].outgoing);
    assert!(MESSAGES[2].text.contains('\n'));
    assert_eq!(MESSAGES[4].time, "10:46");
    assert_eq!(
        MESSAGES[4].text,
        "Il composer è disabilitato: nessun messaggio può essere inviato in questa shell."
    );
}

#[test]
fn shell_tokens_define_baseline_geometry_and_original_palette() {
    let t = matrix_desktop::LIGHT;
    assert_eq!(
        (t.width, t.height, t.sidebar, t.header, t.composer),
        (1120., 760., 320., 64., 72.)
    );
    assert_eq!(t.surface, 0xffffff);
    assert_eq!(t.text, 0x17212b);
    assert_eq!(t.secondary, 0x465b70);
    assert_eq!(t.accent, 0x1264aa);
    assert_eq!(t.timeline, 0xe8f0ed);
    assert_eq!(t.outgoing, 0xd9fdd3);
    assert_eq!(t.selected, 0xe8f3fd);
    assert_eq!(t.border, 0xe3e9ed);
}
