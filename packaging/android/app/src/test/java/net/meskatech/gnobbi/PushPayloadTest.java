package net.meskatech.gnobbi;

import static org.junit.Assert.*;

import java.util.HashMap;
import java.util.Map;
import org.junit.Test;

public class PushPayloadTest {
    private static Map<String, String> m(String... kv) {
        Map<String, String> d = new HashMap<>();
        for (int i = 0; i < kv.length; i += 2) d.put(kv[i], kv[i + 1]);
        return d;
    }

    @Test public void full() {
        PushPayload p = PushPayload.parse(m("gnobbi_account", "@a:x", "room_id", "!r:x", "event_id", "$e"));
        assertEquals("@a:x", p.account);
        assertEquals("!r:x", p.roomId);
        assertEquals("$e", p.eventId);
    }

    @Test public void countsOnlyHasNoEvent() {
        assertEquals("", PushPayload.parse(m("gnobbi_account", "a", "room_id", "r")).eventId);
    }

    @Test public void withoutAccountOrRoomIsIgnored() {
        assertNull(PushPayload.parse(m("room_id", "r")));
        assertNull(PushPayload.parse(m("gnobbi_account", "a")));
        assertNull(PushPayload.parse(m("gnobbi_account", "", "room_id", "r")));
        assertNull(PushPayload.parse(null));
    }
}
