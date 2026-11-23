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
        Intent open = new Intent(ctx, MainActivity.class)
                .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .putExtra(EXTRA_ACCOUNT, p.account)
                .putExtra(EXTRA_ROOM, p.roomId);
        // PendingIntent identity ignores extras: a distinct request code per account+room keeps targets apart.
        int code = (p.account + '\0' + p.roomId).hashCode();
        PendingIntent tap = PendingIntent.getActivity(ctx, code, open,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        // Generic text only: fetching and decrypting the event is t_ec008f7e (never invent content).
        return new Notification.Builder(ctx, CHANNEL_ID)
                .setSmallIcon(R.mipmap.ic_launcher_foreground)
                .setContentTitle(ctx.getString(R.string.app_name))
                .setContentText(ctx.getString(R.string.notification_generic))
                .setContentIntent(tap)
                .setAutoCancel(true)
                .build();
    }

    /** Returns false (nothing posted) when the user refused POST_NOTIFICATIONS or disabled notifications. */
    static boolean show(Context ctx, PushPayload p) {
        NotificationManager nm = ctx.getSystemService(NotificationManager.class);
        if (nm == null || !nm.areNotificationsEnabled()) return false;
        if (Build.VERSION.SDK_INT >= 33
                && ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            return false;
        }
        nm.createNotificationChannel(new NotificationChannel(CHANNEL_ID,
                ctx.getString(R.string.channel_messages), NotificationManager.IMPORTANCE_HIGH));
        nm.notify((p.account + '\0' + p.roomId).hashCode(), build(ctx, p));
        return true;
    }
}
