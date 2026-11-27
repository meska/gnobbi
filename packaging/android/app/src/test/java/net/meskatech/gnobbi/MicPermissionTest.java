package net.meskatech.gnobbi;

import static org.junit.Assert.*;

import android.Manifest;
import android.content.pm.PackageManager;
import android.os.Looper;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.Robolectric;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.Shadows;
import org.robolectric.shadows.ShadowActivity;
import org.robolectric.annotation.Config;

/** RECORD_AUDIO: asked on the microphone tap (MainActivity.requestMic, called by the engine), never at start-up. */
@RunWith(RobolectricTestRunner.class)
@Config(sdk = 35)
public class MicPermissionTest {
    private static final int MIC = 2; // MainActivity.MIC

    // get(), not create(): onCreate loads the native engine, which the JVM does not have.
    private final MainActivity activity = Robolectric.buildActivity(MainActivity.class).get();
    private final ShadowActivity shadow = Shadows.shadowOf(activity);

    private static void idle() { Shadows.shadowOf(Looper.getMainLooper()).idle(); }

    @Test public void notAskedUntilTheFirstTap() {
        idle();
        assertNull(shadow.getLastRequestedPermission());
    }

    @Test public void firstTapRaisesThePromptAndAnswersNotYet() {
        Shadows.shadowOf(activity.getApplication()).denyPermissions(Manifest.permission.RECORD_AUDIO);
        assertFalse(activity.requestMic());
        idle();
        ShadowActivity.PermissionsRequest r = shadow.getLastRequestedPermission();
        assertNotNull(r);
        assertArrayEquals(new String[] {Manifest.permission.RECORD_AUDIO}, r.requestedPermissions);
        assertEquals(MIC, r.requestCode);
    }

    @Test public void alreadyGrantedAnswersYesWithoutAPrompt() {
        Shadows.shadowOf(activity.getApplication()).grantPermissions(Manifest.permission.RECORD_AUDIO);
        assertTrue(activity.requestMic());
        idle();
        assertNull(shadow.getLastRequestedPermission());
    }

    @Test public void everyTapWhileDeniedAsksAgainSoAPermanentDenialStaysAnswered() {
        // «Don't ask again» makes the system answer «denied» at once: the engine must get that answer on every tap.
        Shadows.shadowOf(activity.getApplication()).denyPermissions(Manifest.permission.RECORD_AUDIO);
        for (int i = 0; i < 3; i++) {
            assertFalse(activity.requestMic());
            idle();
            assertNotNull(shadow.getLastRequestedPermission());
        }
    }

    @Test public void answersOfThePromptDoNotThrowAndDoNotTouchTheNotificationPath() {
        int[] granted = {PackageManager.PERMISSION_GRANTED}, denied = {PackageManager.PERMISSION_DENIED};
        String[] p = {Manifest.permission.RECORD_AUDIO};
        activity.onRequestPermissionsResult(MIC, p, granted);
        activity.onRequestPermissionsResult(MIC, p, denied);
        activity.onRequestPermissionsResult(MIC, new String[0], new int[0]); // prompt cancelled: empty arrays = denial
    }
}
