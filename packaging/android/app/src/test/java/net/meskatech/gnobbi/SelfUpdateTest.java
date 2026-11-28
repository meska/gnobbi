package net.meskatech.gnobbi;

import static org.junit.Assert.*;

import android.app.Application;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.Signature;
import android.content.pm.SigningInfo;
import android.net.Uri;
import android.provider.Settings;
import androidx.test.core.app.ApplicationProvider;
import java.io.File;
import java.nio.file.Files;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.Shadows;
import org.robolectric.annotation.Config;
import org.robolectric.shadow.api.Shadow;
import org.robolectric.shadows.ShadowPackageManager;
import org.robolectric.shadows.ShadowSigningInfo;

/** The signer check and the system installer hand-off of the `direct` flavor. Certificates are opaque bytes: only equality matters. */
@RunWith(RobolectricTestRunner.class)
@Config(sdk = 35)
public class SelfUpdateTest {
    private final Context ctx = ApplicationProvider.getApplicationContext();
    private final ShadowPackageManager pm = Shadows.shadowOf(ctx.getPackageManager());
    private File apk;

    static Signature cert(int n) {
        return new Signature(new byte[] {1, 2, 3, (byte) n});
    }

    @SuppressWarnings("deprecation")
    static PackageInfo info(String pkg, Signature... certs) {
        PackageInfo i = new PackageInfo();
        i.packageName = pkg;
        i.applicationInfo = new android.content.pm.ApplicationInfo();
        i.applicationInfo.packageName = pkg;
        if (android.os.Build.VERSION.SDK_INT >= 28) {
            SigningInfo s = Shadow.newInstanceOf(SigningInfo.class);
            ((ShadowSigningInfo) Shadow.extract(s)).setSignatures(certs);
            i.signingInfo = s;
        } else {
            i.signatures = certs;
        }
        return i;
    }

    /** FileProvider caches its folder map per process, and every test has its own files dir: leave nothing for the next test class. */
    @After public void forgetProviderFolders() {
        ((java.util.Map<?, ?>) org.robolectric.util.ReflectionHelpers.getStaticField(androidx.core.content.FileProvider.class, "sCache")).clear();
    }

    @Before public void stagedApk() throws Exception {
        forgetProviderFolders();
        File dir = new File(ctx.getFilesDir(), ".cache/gnobbi/updates");
        assertTrue(dir.mkdirs() || dir.isDirectory());
        apk = new File(dir, "gnobbi-9.apk");
        Files.write(apk.toPath(), new byte[] {9, 9});
        // The installed app keeps its manifest (the FileProvider is declared there); only the signer is set.
        PackageInfo mine = pm.getInternalMutablePackageInfo(ctx.getPackageName());
        PackageInfo signed = info(ctx.getPackageName(), cert(1));
        if (android.os.Build.VERSION.SDK_INT >= 28) mine.signingInfo = signed.signingInfo;
        else mine.signatures = signed.signatures;
        pm.setCanRequestPackageInstalls(true);
    }

    private void downloaded(PackageInfo i) {
        pm.setPackageArchiveInfo(apk.getPath(), i);
    }

    private Intent started() {
        return Shadows.shadowOf((Application) ctx).getNextStartedActivity();
    }

    @Test public void sameCertificateOpensTheSystemInstaller() {
        downloaded(info(ctx.getPackageName(), cert(1)));
        assertEquals(SelfUpdate.OPENED, SelfUpdate.install(ctx, apk.getPath()));
        Intent view = started();
        assertEquals(Intent.ACTION_VIEW, view.getAction());
        assertEquals("application/vnd.android.package-archive", view.getType());
        assertTrue((view.getFlags() & Intent.FLAG_GRANT_READ_URI_PERMISSION) != 0);
        assertEquals("net.meskatech.gnobbi.files", view.getData().getAuthority());
        assertTrue(apk.exists());
    }

    @Test public void anotherCertificateIsRefusedAndTheFileDeleted() {
        downloaded(info(ctx.getPackageName(), cert(2)));
        assertEquals(SelfUpdate.SIGNATURE, SelfUpdate.install(ctx, apk.getPath()));
        assertFalse(apk.exists());
        assertNull(started());
    }

    @Test public void aSubsetOrSupersetOfSignersIsNotTheSameSigner() {
        downloaded(info(ctx.getPackageName(), cert(1), cert(2)));
        assertEquals(SelfUpdate.SIGNATURE, SelfUpdate.install(ctx, apk.getPath()));
        assertFalse(apk.exists());
    }

    @Test public void anotherPackageWithTheSameCertificateIsRefused() {
        downloaded(info("com.example.other", cert(1)));
        assertEquals(SelfUpdate.SIGNATURE, SelfUpdate.install(ctx, apk.getPath()));
        assertFalse(apk.exists());
    }

    @Test public void anUnreadableApkIsRefused() {
        // no archive info registered: getPackageArchiveInfo returns null, like a truncated or corrupt file
        assertEquals(SelfUpdate.SIGNATURE, SelfUpdate.install(ctx, apk.getPath()));
        assertFalse(apk.exists());
    }

    @Test public void withoutTheInstallPermissionTheSettingsScreenOpensAndTheFileStays() {
        pm.setCanRequestPackageInstalls(false);
        downloaded(info(ctx.getPackageName(), cert(1)));
        assertEquals(SelfUpdate.PERMISSION, SelfUpdate.install(ctx, apk.getPath()));
        Intent settings = started();
        assertEquals(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, settings.getAction());
        assertEquals(Uri.parse("package:" + ctx.getPackageName()), settings.getData());
        assertNull(started());
        assertTrue(apk.exists());
    }

    @Test public void aFileOutsideTheUpdatesFolderIsNeverHandedOver() throws Exception {
        File other = new File(ctx.getFilesDir(), "elsewhere.apk");
        Files.write(other.toPath(), new byte[] {1});
        pm.setPackageArchiveInfo(other.getPath(), info(ctx.getPackageName(), cert(1)));
        assertEquals(SelfUpdate.UNAVAILABLE, SelfUpdate.install(ctx, other.getPath()));
        assertNull(started());
    }

    @Test public void theNativeEntryPointFollowsTheFlavor() {
        downloaded(info(ctx.getPackageName(), cert(2)));
        int expected = BuildConfig.SELF_UPDATE ? SelfUpdate.SIGNATURE : SelfUpdate.UNAVAILABLE; // play: nothing is checked, nothing opens
        assertEquals(expected, Native.installApk(ctx, apk.getPath()));
        assertEquals(!BuildConfig.SELF_UPDATE, apk.exists());
        assertEquals(BuildConfig.SELF_UPDATE, Native.selfUpdate());
    }
}
