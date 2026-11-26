package dev.yoloforge.mobile;

import com.google.firebase.messaging.FirebaseMessagingService;
import com.google.firebase.messaging.RemoteMessage;

public final class InboxPushService extends FirebaseMessagingService {
    @Override public void onMessageReceived(RemoteMessage message) {
        InboxPush.show(this, message.getData());
    }

    @Override public void onNewToken(String token) {
        InboxPush.refresh(this, false, null);
    }
}
