package dev.yoloforge.mobile;

import android.app.Application;

public final class YoloApplication extends Application {
    @Override public void onCreate() {
        super.onCreate();
        InboxPush.restore(this);
    }
}
