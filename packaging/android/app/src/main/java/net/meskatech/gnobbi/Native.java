package net.meskatech.gnobbi;

import android.content.Context;
import android.util.Log;

/** Bridge to libgnobbi.so: Java_net_meskatech_gnobbi_Native_nativeOnToken / _nativeOnOpen in crates/matrix-desktop/src/push.rs. */
final class Native {
    private Native() {}

    private static native void nativeOnPermission();

    private static native void nativeOnMicPermission(boolean granted);

    private static native void nativeOnPicked(String paths);

    private static native void nativeOnToken(String token);

    private static native void nativeOnOpen(String account, String room);

    private static native boolean nativeIsRoomVisible(String account, String room);

    private static native String nativeAlert(Context ctx, String filesDir, String account, String room, String event);

    /**
     * What to post for a push, from the engine (fetch + decrypt, preview and switch rules in Rust): "" is the generic text, also when
     * the library is missing or the engine fails. Blocks up to about 8 s; call it from onMessageReceived only.
     */
    static String alert(Context ctx, PushPayload p) {
        try {
            System.loadLibrary("gnobbi");
            String a = nativeAlert(ctx.getApplicationContext(), ctx.getFilesDir().getCanonicalPath(), p.account, p.roomId, p.eventId == null ? "" : p.eventId); // "": a null would fail before the engine checks the switch
            return a == null ? "" : a;
        } catch (UnsatisfiedLinkError | java.io.IOException e) {
            return "";
        }
    }

    /** True when the user is reading that room right now; false (show the notification) when the engine is not loaded. */
    static boolean isRoomVisible(String account, String room) {
        try {
            System.loadLibrary("gnobbi");
            return nativeIsRoomVisible(account, room);
        } catch (UnsatisfiedLinkError e) {
            return false;
        }
    }

    /** Hands the FCM token to the engine (first token and every onNewToken); a missing library must not crash the service. */
    static void onToken(String token) {
        try {
            System.loadLibrary("gnobbi");
            nativeOnToken(token);
        } catch (UnsatisfiedLinkError e) {
            Log.w("Gnobbi", "token not delivered to the engine: native side unavailable");
        }
    }

    /** Tells the engine which account and room a notification tap asked for (cold start, background or foreground). */
    static void onOpen(String account, String room) {
        try {
            System.loadLibrary("gnobbi");
            nativeOnOpen(account, room);
        } catch (UnsatisfiedLinkError e) {
            Log.w("Gnobbi", "tap not delivered to the engine: native side unavailable");
        }
    }

    /** POST_NOTIFICATIONS is granted: the engine switches Notifications on if the user never chose. */
    static void onPermission() {
        try {
            System.loadLibrary("gnobbi");
            nativeOnPermission();
        } catch (UnsatisfiedLinkError e) {
            Log.w("Gnobbi", "permission not delivered to the engine: native side unavailable");
        }
    }

    /** The files copied from the system picker (none: closed); the engine reads them and adds them to the preview. */
    static void onPicked(String[] paths) {
        try {
            System.loadLibrary("gnobbi");
            nativeOnPicked(String.join("\n", paths));
        } catch (UnsatisfiedLinkError e) {
            Log.w("Gnobbi", "pick not delivered to the engine: native side unavailable");
        }
    }

    /** The answer to the RECORD_AUDIO prompt raised by the first tap on the microphone. */
    static void onMicPermission(boolean granted) {
        try {
            System.loadLibrary("gnobbi");
            nativeOnMicPermission(granted);
        } catch (UnsatisfiedLinkError e) {
            Log.w("Gnobbi", "mic permission not delivered to the engine: native side unavailable");
        }
    }
}
