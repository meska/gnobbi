package net.meskatech.gnobbi;

import static org.junit.Assert.*;

import android.app.Notification;
import android.content.Context;
import android.content.Intent;
import androidx.test.core.app.ApplicationProvider;
import java.util.HashMap;
import java.util.Map;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.Shadows;
import org.robolectric.annotation.Config;

@RunWith(RobolectricTestRunner.class)
@Config(sdk = 35)
public class NotifierTest {
    private final Context ctx = ApplicationProvider.getApplicationContext();

    private static PushPayload sample(String account, String room) {
        Map<String, String> d = new HashMap<>();
        d.put("gnobbi_account", account);
        d.put("room_id", room);
        d.put("event_id", "$e");
        return PushPayload.parse(d);
    }

    @Test public void tapOpensMainActivityWithAccountAndRoom() {
        Notification n = Notifier.build(ctx, sample("@a:x", "!r:x"));
        Intent i = Shadows.shadowOf(n.contentIntent).getSavedIntent();
        assertEquals(MainActivity.class.getName(), i.getComponent().getClassName());
        assertEquals("@a:x", i.getStringExtra(Notifier.EXTRA_ACCOUNT));
        assertEquals("!r:x", i.getStringExtra(Notifier.EXTRA_ROOM));
        assertEquals(Notifier.CHANNEL_ID, n.getChannelId());
    }

    @Test public void validPushPostsNotificationAndChannel() {
        android.app.NotificationManager nm = ctx.getSystemService(android.app.NotificationManager.class);
        Shadows.shadowOf((android.app.Application) ctx).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS);
        assertTrue(Notifier.show(ctx, sample("@a:x", "!r:x"), false));
        assertEquals(1, Shadows.shadowOf(nm).size());
        assertNotNull(nm.getNotificationChannel(Notifier.CHANNEL_ID));
    }

    @Test public void deniedRuntimePermissionPostsNothing() {
        Shadows.shadowOf((android.app.Application) ctx).denyPermissions(android.Manifest.permission.POST_NOTIFICATIONS);
        assertFalse(Notifier.show(ctx, sample("@a:x", "!r:x"), false));
    }

    @Test public void disabledNotificationsPostNothing() {
        android.app.NotificationManager nm = ctx.getSystemService(android.app.NotificationManager.class);
        Shadows.shadowOf(nm).setNotificationsEnabled(false);
        assertFalse(Notifier.show(ctx, sample("@a:x", "!r:x"), false));
        assertEquals(0, Shadows.shadowOf(nm).size());
    }

    @Test public void tagMatchesTheEnginesRoomKeyAndIsPerRoom() {
        assertEquals("3:7:!r:x", Notifier.tag(sample("3:7:accounts/3", "!r:x")));
        assertNotEquals(Notifier.tag(sample("3:7:accounts/3", "!r:x")), Notifier.tag(sample("3:7:accounts/3", "!s:x")));
        assertNotEquals(Notifier.tag(sample("3:7:accounts/3", "!r:x")), Notifier.tag(sample("4:7:accounts/4", "!r:x")));
    }

    @Test public void sameRoomReplacesAndAnotherRoomAddsAndCancelByTagRemoves() {
        android.app.NotificationManager nm = ctx.getSystemService(android.app.NotificationManager.class);
        Shadows.shadowOf((android.app.Application) ctx).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS);
        Notifier.show(ctx, sample("3:7:a", "!r:x"), false);
        Notifier.show(ctx, sample("3:7:a", "!r:x"), false);
        Notifier.show(ctx, sample("3:7:a", "!s:x"), false);
        assertEquals(2, Shadows.shadowOf(nm).size());
        nm.cancel("3:7:!r:x", 0); // what the engine's clear_rooms does when the room is read
        assertEquals(1, Shadows.shadowOf(nm).size());
    }

    @Test public void roomOnScreenPostsNothing() {
        android.app.NotificationManager nm = ctx.getSystemService(android.app.NotificationManager.class);
        Shadows.shadowOf((android.app.Application) ctx).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS);
        assertFalse(Notifier.show(ctx, sample("3:7:a", "!r:x"), true));
        assertEquals(0, Shadows.shadowOf(nm).size());
    }

    @Test public void withoutTheNativeLibraryTheNotificationIsShown() {
        assertFalse(Native.isRoomVisible("3:7:a", "!r:x"));
    }
}
