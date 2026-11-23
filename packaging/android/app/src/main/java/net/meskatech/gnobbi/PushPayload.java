package net.meskatech.gnobbi;

import java.util.Map;

/** Data fields of a Gnobbi push (event_id_only + gnobbi_account default payload). */
final class PushPayload {
    final String account;
    final String roomId;
    final String eventId; // may be empty: counts-only push

    private PushPayload(String account, String roomId, String eventId) {
        this.account = account;
        this.roomId = roomId;
        this.eventId = eventId;
    }

    /** Null when the push cannot be tied to an account and a room: nothing safe to show or open. */
    static PushPayload parse(Map<String, String> data) {
        if (data == null) return null;
        String account = data.get("gnobbi_account");
        String room = data.get("room_id");
        if (account == null || account.isEmpty() || room == null || room.isEmpty()) return null;
        String event = data.get("event_id");
        return new PushPayload(account, room, event == null ? "" : event);
    }
}
