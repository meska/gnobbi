package net.meskatech.gnobbi;

import android.util.Log;

/** Bridge to libgnobbi.so. The Rust side (Java_net_meskatech_gnobbi_Native_onToken) lands with t_ec008f7e. */
final class Native {
    private Native() {}

    private static native void nativeOnToken(String token);

    /** Hands the FCM token to the engine; a missing library or symbol must not crash the service. */
    static void onToken(String token) {
        try {
            System.loadLibrary("gnobbi");
            nativeOnToken(token);
        } catch (UnsatisfiedLinkError e) {
            Log.w("Gnobbi", "token not delivered to the engine: native side unavailable");
        }
    }
}
