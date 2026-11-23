package net.meskatech.gnobbi;

import android.Manifest;
import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;

public class MainActivity extends NativeActivity {
    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        // Cold start only; after two refusals Android stops showing the dialog by itself.
        if (state == null && Build.VERSION.SDK_INT >= 33
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS}, 1);
        }
    }

    // Refusal needs no handling here: Notifier.show() re-checks the permission and drops the push silently.

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent); // singleTop: the native side reads account/room from getIntent()
    }
}
