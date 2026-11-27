package net.meskatech.gnobbi;

import android.Manifest;
import android.app.NativeActivity;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.webkit.MimeTypeMap;
import android.view.View;
import android.view.Window;
import android.view.WindowInsetsController;
import androidx.core.content.FileProvider;
import com.google.firebase.messaging.FirebaseMessaging;
import java.io.File;
import java.util.Locale;

public class MainActivity extends NativeActivity {
    /** True between onResume and onPause: after onPause no frame runs, so the engine's «room visible» flag would stay stale. */
    static volatile boolean resumed;

    @Override
    protected void onResume() {
        super.onResume();
        resumed = true;
    }

    @Override
    protected void onPause() {
        resumed = false;
        super.onPause();
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        if (Build.VERSION.SDK_INT < 33 || checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) {
            Native.onPermission();
        } else if (state == null) { // cold start only; after two refusals Android stops showing the dialog by itself
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

    @Override
    public void onRequestPermissionsResult(int code, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(code, permissions, results);
        if (results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED) Native.onPermission();
        // Refusal needs no handling here: Notifier.show() re-checks the permission and drops the push silently.
    }

    /** Called by the engine over JNI with the app theme: dark status/navigation icons on a light theme, light icons on a dark one. */
    public void setBarIcons(boolean lightTheme) {
        runOnUiThread(() -> {
            Window w = getWindow();
            if (Build.VERSION.SDK_INT >= 30) {
                WindowInsetsController c = w.getInsetsController();
                int mask = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                if (c != null) c.setSystemBarsAppearance(lightTheme ? mask : 0, mask);
            } else {
                View d = w.getDecorView();
                int mask = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR; // minSdk 26: both exist
                d.setSystemUiVisibility(lightTheme ? d.getSystemUiVisibility() | mask : d.getSystemUiVisibility() & ~mask);
            }
        });
    }

    /** Called by the engine over JNI: the received file (with the attachment's name) goes to the system chooser. */
    public boolean shareFile(String path, String name) {
        return share(this, path, name);
    }

    /**
     * The system chooser offers a received file of the engine's temp folder (`video-tmp`, nothing else of the data folder) to other
     * apps through a FileProvider URI named `name` (read access for the chosen app only; the engine deletes the file as for «Open»).
     */
    static boolean share(Context context, String path, String name) {
        try {
            File file = new File(path).getCanonicalFile();
            File dir = file.getParentFile();
            if (dir == null || !"video-tmp".equals(dir.getName())) return false;
            String ext = MimeTypeMap.getFileExtensionFromUrl(name);
            String mime = ext == null ? null : MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext.toLowerCase(Locale.ROOT));
            Uri uri = FileProvider.getUriForFile(context, "net.meskatech.gnobbi.files", file, name);
            Intent send = new Intent(Intent.ACTION_SEND)
                .setType(mime == null ? "application/octet-stream" : mime)
                .putExtra(Intent.EXTRA_STREAM, uri)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            context.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            return true;
        } catch (java.io.IOException | RuntimeException e) { // a path outside the provider's folders, or no app to take it
            Log.w("Gnobbi", "share failed: " + e.getClass().getSimpleName());
            return false;
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        deliverTap(intent);
    }
}
