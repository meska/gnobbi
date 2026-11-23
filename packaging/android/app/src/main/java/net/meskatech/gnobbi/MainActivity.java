package net.meskatech.gnobbi;

import android.Manifest;
import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import com.google.firebase.messaging.FirebaseMessaging;

public class MainActivity extends NativeActivity {
    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        // Cold start only; after two refusals Android stops showing the dialog by itself.
        if (state == null && Build.VERSION.SDK_INT >= 33
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS}, 1);
        }
        fetchToken();
        if (state == null) deliverTap(getIntent()); // a recreated activity must not reopen the room
    }

    /** onNewToken fires only on rotation: after a process restart the engine needs the current token from here. */
    private static void fetchToken() {
        try {
            FirebaseMessaging.getInstance().getToken().addOnCompleteListener(task -> {
                if (task.isSuccessful() && task.getResult() != null) Native.onToken(task.getResult());
                else Log.w("Gnobbi", "FCM getToken failed: " + (task.getException() == null ? "no token" : task.getException().getClass().getSimpleName())); // class only: no token, no message
            });
        } catch (RuntimeException e) {
            Log.w("Gnobbi", "no FCM token: Firebase not configured or Google Play services missing"); // no push, but no crash
        }
    }

    /** The extras come from any app (the activity is exported): the engine validates them (shape, known account, known room). */
    private static void deliverTap(Intent intent) {
        String account = intent.getStringExtra(Notifier.EXTRA_ACCOUNT);
        String room = intent.getStringExtra(Notifier.EXTRA_ROOM);
        if (account == null || room == null) return;
        intent.removeExtra(Notifier.EXTRA_ACCOUNT);
        intent.removeExtra(Notifier.EXTRA_ROOM);
        Native.onOpen(account, room);
    }

    // Refusal needs no handling here: Notifier.show() re-checks the permission and drops the push silently.

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        deliverTap(intent);
    }
}
