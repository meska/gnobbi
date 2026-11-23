package net.meskatech.gnobbi;

import com.google.firebase.messaging.FirebaseMessagingService;
import com.google.firebase.messaging.RemoteMessage;

public final class GnobbiPushService extends FirebaseMessagingService {
    @Override
    public void onNewToken(String token) {
        Native.onToken(token);
    }

    @Override
    public void onMessageReceived(RemoteMessage message) {
        PushPayload p = PushPayload.parse(message.getData());
        if (p != null) Notifier.show(this, p, Native.isRoomVisible(p.account, p.roomId));
    }
}
