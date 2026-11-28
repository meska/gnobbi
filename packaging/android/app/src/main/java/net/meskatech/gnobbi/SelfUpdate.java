package net.meskatech.gnobbi;

import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.net.Uri;
import android.os.Build;
import android.provider.Settings;
import android.util.Log;
import androidx.core.content.FileProvider;
import java.io.File;
import java.io.IOException;
import java.util.HashSet;
import java.util.Set;

/** The `direct` flavor's self-update: the Rust updater has already checked size and SHA-256, this checks the signer and hands the APK to the system installer. */
final class SelfUpdate {
    private SelfUpdate() {}

    // Codes read by crates/matrix-desktop/src/update/android.rs.
    static final int OPENED = 0;
    static final int SIGNATURE = 1;
    static final int PERMISSION = 2;
    static final int UNAVAILABLE = 3;

    private static final String APK = "application/vnd.android.package-archive";

    static int install(Context ctx, String path) {
        try {
            File apk = new File(path).getCanonicalFile();
            if (!sameSigner(ctx, apk)) {
                apk.delete(); // an APK from another signer must not stay around to be installed by hand
                return SIGNATURE;
            }
            if (!ctx.getPackageManager().canRequestPackageInstalls()) {
                ctx.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + ctx.getPackageName())).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
                return PERMISSION;
            }
            Uri uri = FileProvider.getUriForFile(ctx, "net.meskatech.gnobbi.files", apk); // only the updates/ folder is mapped for this
            ctx.startActivity(new Intent(Intent.ACTION_VIEW).setDataAndType(uri, APK).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_ACTIVITY_NEW_TASK));
            return OPENED;
        } catch (IOException | RuntimeException e) { // a path outside the provider's folders, or no installer to take it
            Log.w("Gnobbi", "update install failed: " + e.getClass().getSimpleName());
            return UNAVAILABLE;
        }
    }

    /** The downloaded APK is signed with the certificate of the installed app, and is that app: the only trust anchor (the manifest is not compared). */
    static boolean sameSigner(Context ctx, File apk) {
        PackageManager pm = ctx.getPackageManager();
        int flags = Build.VERSION.SDK_INT >= 28 ? PackageManager.GET_SIGNING_CERTIFICATES : PackageManager.GET_SIGNATURES;
        try {
            PackageInfo mine = pm.getPackageInfo(ctx.getPackageName(), flags);
            PackageInfo theirs = pm.getPackageArchiveInfo(apk.getPath(), flags);
            if (theirs == null || !ctx.getPackageName().equals(theirs.packageName)) return false;
            Set<Signature> a = signers(mine);
            return !a.isEmpty() && a.equals(signers(theirs));
        } catch (PackageManager.NameNotFoundException | RuntimeException e) {
            return false;
        }
    }

    @SuppressWarnings("deprecation") // GET_SIGNATURES is the only way on API 26-27
    private static Set<Signature> signers(PackageInfo info) {
        Signature[] s = Build.VERSION.SDK_INT >= 28 ? (info.signingInfo == null ? null : info.signingInfo.getApkContentsSigners()) : info.signatures;
        Set<Signature> out = new HashSet<>();
        if (s != null) for (Signature x : s) out.add(x);
        return out;
    }
}
