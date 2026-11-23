package net.meskatech.gnobbi;

import android.util.Log;

/** Bridge to libgnobbi.so: Java_net_meskatech_gnobbi_Native_nativeOnToken / _nativeOnOpen in crates/matrix-desktop/src/push.rs. */
final class Native {
    private Native() {}

    private static native void nativeOnToken(String token);

    private static native void nativeOnOpen(String account, String room);

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
}
