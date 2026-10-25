use matrix_desktop::HistoryStatus;

#[test]
fn older_history_strip_never_shows_the_shortcut_or_idle_text() {
    for idle in [HistoryStatus::Empty, HistoryStatus::More, HistoryStatus::Start] {
        assert_eq!(idle.strip_label(), None);
    }
    for shown in [HistoryStatus::Loading, HistoryStatus::Failed] {
        let text = shown.strip_label().expect("strip text");
        assert!(!text.contains("Ctrl") && !text.contains("PageUp"), "{text}");
    }
}
