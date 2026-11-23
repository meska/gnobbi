package net.meskatech.gnobbi;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;

/** Builds and posts the local notification; the tap opens MainActivity with account and room as extras. */
final class Notifier {
    static final String CHANNEL_ID = "messages";
    static final String EXTRA_ACCOUNT = "gnobbi_account";
    static final String EXTRA_ROOM = "room_id";

    private Notifier() {}

    static Notification build(Context ctx, PushPayload p) {
        return build(ctx, p, "");
    }

    /** {@code alert} is the engine's answer (see Alert::wire in push.rs): "" generic, "O" + sender, "T" + sender + newline + text. */
    static Notification build(Context ctx, PushPayload p, String alert) {
        Intent open = new Intent(ctx, MainActivity.class)
                .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .putExtra(EXTRA_ACCOUNT, p.account)
                .putExtra(EXTRA_ROOM, p.roomId);
        // PendingIntent identity ignores extras: a distinct request code per account+room keeps targets apart.
        int code = (p.account + '\0' + p.roomId).hashCode();
        PendingIntent tap = PendingIntent.getActivity(ctx, code, open,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        // Content only when the engine read it; anything else keeps the generic text (never invent content).
        String title = ctx.getString(R.string.app_name);
        String text = ctx.getString(R.string.notification_generic);
        int nl = alert.indexOf('\n');
        if (alert.startsWith("T") && nl > 1) {
            title = alert.substring(1, nl);
            text = alert.substring(nl + 1);
        } else if (alert.startsWith("O") && alert.length() > 1) {
            title = alert.substring(1);
        }
        return new Notification.Builder(ctx, CHANNEL_ID)
                .setSmallIcon(R.mipmap.ic_launcher_foreground)
                .setContentTitle(title)
                .setContentText(text)
                .setContentIntent(tap)
                .setAutoCancel(true)
                .build();
    }

    /**
     * One notification per room, tagged like the engine's cancel in notifications.rs (target_key): "catalog:worker:room".
     * The account extra is "catalog:worker:dir"; a malformed one falls back to itself so it still cannot collide across rooms.
     */
    static String tag(PushPayload p) {
        String[] parts = p.account.split(":", 3);
        return (parts.length == 3 ? parts[0] + ':' + parts[1] : p.account) + ':' + p.roomId;
    }

    /**
     * Returns false (nothing posted) when the room is on screen in a focused window ({@code roomVisible}, asked to the engine),
     * the user refused POST_NOTIFICATIONS or notifications are disabled.
     */
    static boolean show(Context ctx, PushPayload p, boolean roomVisible) {
        return show(ctx, p, roomVisible, "");
    }

    /** Also false for the engine's "S" (Notifications switch off, or an edit/redaction: nothing to alert). */
    static boolean show(Context ctx, PushPayload p, boolean roomVisible, String alert) {
        if (roomVisible || alert.equals("S")) return false;
        NotificationManager nm = ctx.getSystemService(NotificationManager.class);
        if (nm == null || !nm.areNotificationsEnabled()) return false;
        if (Build.VERSION.SDK_INT >= 33
                && ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            return false;
        }
        nm.createNotificationChannel(new NotificationChannel(CHANNEL_ID,
                ctx.getString(R.string.channel_messages), NotificationManager.IMPORTANCE_HIGH));
        nm.notify(tag(p), 0, build(ctx, p, alert));
        return true;
    }
}
