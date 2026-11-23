package net.meskatech.gnobbi;

import com.google.firebase.messaging.FirebaseMessagingService;
import com.google.firebase.messaging.RemoteMessage;

public final class GnobbiPushService extends FirebaseMessagingService {
    @Override
    public void onNewToken(String token) {
        Native.onToken(token);
    }

    /** Only a foreground activity can show the room: the engine's flag is not cleared when the app goes to the background. */
    static boolean roomOnScreen(boolean resumed, boolean engineSaysVisible) {
        return resumed && engineSaysVisible;
    }

    @Override
    public void onMessageReceived(RemoteMessage message) {
        PushPayload p = PushPayload.parse(message.getData());
        if (p == null || roomOnScreen(MainActivity.resumed, Native.isRoomVisible(p.account, p.roomId))) return;
        Notifier.show(this, p, false, Native.alert(this, p));
    }
}
