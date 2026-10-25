/// Gap after which the same sender's next message gets the name again.
pub const SENDER_LABEL_GAP_MS: u64 = 5 * 60_000;

/// Whether a bubble shows the sender name: only incoming messages in rooms with
/// more than two members, and only when the previous message has a different
/// sender or is older than `SENDER_LABEL_GAP_MS`.
/// `previous` is `(same_sender_as_this, age_of_previous_ms)`.
pub fn shows_sender_label(member_count: u32, outgoing: bool, previous: Option<(bool, u64)>) -> bool {
    if outgoing || member_count <= 2 {
        return false;
    }
    match previous {
        Some((same_sender, gap_ms)) => !same_sender || gap_ms > SENDER_LABEL_GAP_MS,
        None => true,
    }
}
