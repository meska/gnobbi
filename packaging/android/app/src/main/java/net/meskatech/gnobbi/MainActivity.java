package net.meskatech.gnobbi;

import android.Manifest;
import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.database.Cursor;
import android.os.Bundle;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import android.util.Log;
import android.webkit.MimeTypeMap;
import android.view.View;
import android.view.Window;
import android.view.WindowInsetsController;
import androidx.core.content.FileProvider;
import com.google.firebase.messaging.FirebaseMessaging;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;
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

    private static final int PICK = 0x4750;
    /** The preview takes up to 10 files (attach.rs MAX_ITEMS). */
    private static final int PICK_MAX = 10;
    private long pickLimit;

    /**
     * Called by the engine over JNI: the system pickers, no storage permission. mode 0: any file, 1: photo or video, 2: picture.
     * Photo Picker (Android 13+) for 1 and 2, the documents picker for 0 and where there is no Photo Picker. The answer comes
     * from onActivityResult as Native.onPicked: the app's copies of the files, none when the dialog was closed.
     */
    public boolean pickFiles(int mode, boolean many, long limit) {
        runOnUiThread(() -> {
            pickLimit = limit;
            if (mode != 0 && Build.VERSION.SDK_INT >= 33) {
                Intent photos = new Intent(MediaStore.ACTION_PICK_IMAGES);
                if (mode == 2) photos.setType("image/*");
                if (many) photos.putExtra(MediaStore.EXTRA_PICK_IMAGES_MAX, Math.min(PICK_MAX, MediaStore.getPickImagesMaxLimit()));
                try {
                    startActivityForResult(photos, PICK);
                    return;
                } catch (ActivityNotFoundException | SecurityException e) {
                    Log.w("Gnobbi", "no photo picker: " + e.getClass().getSimpleName());
                }
            }
            Intent docs = new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*").putExtra(Intent.EXTRA_ALLOW_MULTIPLE, many);
            if (mode != 0) docs.putExtra(Intent.EXTRA_MIME_TYPES, mode == 2 ? new String[] {"image/*"} : new String[] {"image/*", "video/*"});
            try {
                startActivityForResult(docs, PICK);
            } catch (ActivityNotFoundException e) {
                Native.onPicked(new String[0]);
            }
        });
        return true;
    }

    @Override
    protected void onActivityResult(int code, int result, Intent data) {
        super.onActivityResult(code, result, data);
        if (code != PICK) return;
        List<Uri> uris = new ArrayList<>();
        if (result == RESULT_OK && data != null) {
            ClipData clip = data.getClipData();
            if (clip != null) {
                for (int i = 0; i < clip.getItemCount() && uris.size() < PICK_MAX; i++) uris.add(clip.getItemAt(i).getUri());
            } else if (data.getData() != null) {
                uris.add(data.getData());
            }
        }
        long limit = pickLimit;
        ContentResolver resolver = getContentResolver();
        File root = new File(getCacheDir(), "picked");
        new Thread(() -> Native.onPicked(copyPicked(resolver, root, uris, limit))).start(); // a file may be as big as the server's limit
    }

    /**
     * Copies each picked item into its own folder of `root` under its display name, at most `limit` bytes each; the previous pick's
     * copies are deleted first (the engine has read them by now). An item that cannot be read or is over the limit leaves an empty
     * file, which the preview reports as «File cannot be attached». Returns one path per item, in order.
     */
    static String[] copyPicked(ContentResolver resolver, File root, List<Uri> uris, long limit) {
        deleteTree(root);
        String[] paths = new String[uris.size()];
        for (int i = 0; i < paths.length; i++) {
            File dir = new File(root, String.valueOf(i));
            dir.mkdirs();
            File file = new File(dir, displayName(resolver, uris.get(i)));
            paths[i] = file.getPath();
            long total = 0;
            try (InputStream in = resolver.openInputStream(uris.get(i)); OutputStream out = new FileOutputStream(file)) {
                byte[] buf = new byte[64 * 1024];
                for (int n; in != null && (n = in.read(buf)) > 0; ) {
                    total += n;
                    if (total > limit) break;
                    out.write(buf, 0, n);
                }
            } catch (IOException | RuntimeException e) {
                total = limit + 1;
            }
            if (total > limit) {
                try { new FileOutputStream(file).close(); } catch (IOException e) { Log.w("Gnobbi", "pick: " + e.getClass().getSimpleName()); }
            }
        }
        return paths;
    }

    /** The name the provider gives the item, as one safe path component (never empty, hidden or with a separator). */
    static String displayName(ContentResolver resolver, Uri uri) {
        String name = uri.getLastPathSegment();
        try (Cursor c = resolver.query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && c.getString(0) != null) name = c.getString(0);
        } catch (RuntimeException e) {
            Log.w("Gnobbi", "pick name: " + e.getClass().getSimpleName());
        }
        name = name == null ? "" : name.replaceAll("[/\\\\\\n]", "_").replaceFirst("^\\.+", "");
        return name.isEmpty() ? "file" : name;
    }

    private static void deleteTree(File f) {
        File[] kids = f.listFiles();
        if (kids != null) for (File k : kids) deleteTree(k);
        f.delete();
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        deliverTap(intent);
    }
}
