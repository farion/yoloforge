package dev.yoloforge.mobile;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import com.google.android.gms.tasks.Tasks;
import com.google.firebase.FirebaseApp;
import com.google.firebase.FirebaseOptions;
import com.google.firebase.messaging.FirebaseMessaging;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import org.json.JSONObject;

/** One Factory account at a time; never carry a binding over to a new login. */
final class InboxPush {
    private static final String BINDING = "inbox-push-binding";
    private static final String ENABLED = "inbox-push-enabled";
    private static final String CHANNEL = "inbox";
    private static final String PREFIX = "inbox-push:";
    private static final java.util.concurrent.ExecutorService WORKER = Executors.newSingleThreadExecutor();
    private static final Object LOCK = new Object();
    private static String initialized;
    private static final String DEVICES = "/v1/android-push/devices/";
    static final String UNCONFIGURED = "Diese Factory hat Android-Push nicht eingerichtet.";
    static final String ACTIVE = "Android-Push ist aktiv.";

    interface Result { void finished(String message); }

    private static String loginId(JSONObject credentials) {
        return credentials.optString("push_login");
    }

    static boolean permitted(Context context) {
        return (Build.VERSION.SDK_INT < 33 || context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
            == PackageManager.PERMISSION_GRANTED)
            && context.getSystemService(NotificationManager.class).areNotificationsEnabled();
    }

    private static JSONObject current(Vault vault, JSONObject credentials) throws Exception {
        if (credentials == null || loginId(credentials).isEmpty()) return null;
        JSONObject binding = vault.get(BINDING);
        return binding != null && loginId(credentials).equals(binding.optString("login")) ? binding : null;
    }

    private static boolean enabled(Vault vault, JSONObject credentials) throws Exception {
        JSONObject flag = vault.get(ENABLED);
        return flag != null && credentials != null && loginId(credentials).equals(flag.optString("login"))
            && !loginId(credentials).isEmpty();
    }

    static boolean configured(Context context) {
        synchronized (LOCK) {
            try {
                Vault vault = new Vault(context);
                JSONObject credentials = vault.get("credentials");
                return enabled(vault, credentials) && current(vault, credentials) != null;
            } catch (Exception ignored) { return false; }
        }
    }

    private static void initialize(Context context, JSONObject options) throws Exception {
        String identity = options.getString("project_id") + ":" + options.getString("application_id")
            + ":" + options.getString("api_key");
        if (identity.equals(initialized)) return;
        clearFirebase();
        FirebaseApp.initializeApp(context, new FirebaseOptions.Builder()
            .setProjectId(options.getString("project_id"))
            .setApplicationId(options.getString("application_id"))
            .setGcmSenderId(options.getString("sender_id"))
            .setApiKey(options.getString("api_key")).build());
        initialized = identity;
    }

    private static void clearFirebase() {
        try { FirebaseApp.getInstance().delete(); }
        catch (IllegalStateException ignored) { /* No default app yet. */ }
        initialized = null;
    }

    static void restore(Context context) {
        synchronized (LOCK) {
            try {
                Vault vault = new Vault(context);
                JSONObject credentials = vault.get("credentials");
                JSONObject binding = current(vault, credentials);
                if (binding != null && enabled(vault, credentials)) initialize(context, binding.getJSONObject("firebase"));
            } catch (Exception ignored) { /* A foreground refresh can repair the binding. */ }
        }
    }

    private static Api api(JSONObject credentials) throws Exception {
        return new Api(credentials.getString("base"), credentials.getString("token"));
    }

    private static JSONObject withLogin(Vault vault, JSONObject credentials) throws Exception {
        if (loginId(credentials).isEmpty()) {
            credentials.put("push_login", UUID.randomUUID().toString());
            vault.put("credentials", credentials);
        }
        return credentials;
    }

    static void login(Context context, JSONObject credentials) throws Exception {
        final JSONObject previous;
        final JSONObject oldBinding;
        synchronized (LOCK) {
            Vault vault = new Vault(context);
            previous = vault.get("credentials");
            if (previous != null && credentials.getString("base").equals(previous.optString("base"))
                && credentials.getString("token").equals(previous.optString("token"))) {
                withLogin(vault, previous);
                return;
            }
            oldBinding = current(vault, previous);
            credentials.put("push_login", UUID.randomUUID().toString());
            vault.put("credentials", credentials);
            vault.put(ENABLED, null);
            vault.put(BINDING, null);
            cancel(context);
            clearFirebase();
        }
        context.stopService(new Intent(context, EventService.class));
        if (oldBinding != null) WORKER.execute(() -> unregister(previous, oldBinding));
    }

    static void refresh(Context source, boolean turnOn, Result result) {
        Context context = source.getApplicationContext();
        WORKER.execute(() -> {
            try {
                String message = synchronize(context, turnOn);
                if (result != null) result.finished(message);
            } catch (Exception failure) {
                if (result != null) result.finished("Android-Push konnte nicht eingerichtet werden. Prüfe Play-Dienste und Verbindung.");
            }
        });
    }

    private static boolean stillActive(Vault vault, JSONObject credentials) throws Exception {
        JSONObject latest = vault.get("credentials");
        return latest != null && loginId(credentials).equals(loginId(latest)) && enabled(vault, latest);
    }

    private static String synchronize(Context context, boolean turnOn) throws Exception {
        Vault vault = new Vault(context);
        JSONObject credentials;
        synchronized (LOCK) {
            credentials = vault.get("credentials");
            if (credentials == null || !turnOn && !enabled(vault, credentials)) return "";
            withLogin(vault, credentials);
            if (turnOn) vault.put(ENABLED, new JSONObject().put("login", loginId(credentials)));
        }
        if (!permitted(context)) return "Android-Benachrichtigungen sind nicht erlaubt.";
        Api api = api(credentials);
        JSONObject response;
        try { response = api.request("GET", "/v1/android-push/config", null); }
        catch (Api.HttpFailure error) {
            if (error.status != 404) throw error;
            response = new JSONObject();
        }
        if (!response.optBoolean("enabled")) {
            synchronized (LOCK) {
                if (stillActive(vault, credentials)) {
                    vault.put(BINDING, null);
                    cancel(context);
                    clearFirebase();
                }
            }
            return UNCONFIGURED;
        }
        JSONObject options = response.getJSONObject("firebase");
        String sender = options.getString("application_id");
        final com.google.android.gms.tasks.Task<String> getToken;
        final JSONObject binding;
        synchronized (LOCK) {
            if (!stillActive(vault, credentials)) return "";
            JSONObject existing = current(vault, credentials);
            String id = existing != null && sender.equals(existing.getJSONObject("firebase").optString("application_id"))
                ? existing.getString("id") : UUID.randomUUID().toString();
            binding = new JSONObject().put("id", id).put("login", loginId(credentials)).put("firebase", options);
            initialize(context, options);
            getToken = FirebaseMessaging.getInstance().getToken();
        }
        String token = Tasks.await(getToken, 25, TimeUnit.SECONDS);
        synchronized (LOCK) {
            if (!stillActive(vault, credentials)) return "";
            // Persist before PUT so a push arriving just after registration is accepted.
            vault.put(BINDING, binding);
        }
        api.request("PUT", DEVICES + binding.getString("id"), new JSONObject().put("sender", sender).put("token", token));
        synchronized (LOCK) {
            if (stillActive(vault, credentials)) return ACTIVE;
        }
        unregister(credentials, binding);
        return "";
    }

    static void disable(Context context) throws Exception {
        final JSONObject credentials;
        final JSONObject binding;
        synchronized (LOCK) {
            Vault vault = new Vault(context);
            credentials = vault.get("credentials");
            binding = current(vault, credentials);
            vault.put(ENABLED, null);
            vault.put(BINDING, null);
            cancel(context);
            clearFirebase();
        }
        if (binding != null) WORKER.execute(() -> unregister(credentials, binding));
    }

    private static void unregister(JSONObject credentials, JSONObject binding) {
        try { api(credentials).request("DELETE", DEVICES + binding.getString("id"), null); }
        catch (Exception ignored) { /* Local binding is already invalidated. */ }
    }

    private static void cancel(Context context) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        for (var active : manager.getActiveNotifications())
            if (active.getTag() != null && active.getTag().startsWith(PREFIX)) manager.cancel(active.getTag(), active.getId());
    }

    static boolean accepts(Context context, String registration) {
        synchronized (LOCK) {
            try {
                Vault vault = new Vault(context);
                JSONObject credentials = vault.get("credentials");
                JSONObject binding = current(vault, credentials);
                return binding != null && enabled(vault, credentials) && binding.getString("id").equals(registration);
            } catch (Exception ignored) { return false; }
        }
    }

    static void show(Context context, Map<String, String> data) {
        synchronized (LOCK) {
            String registration = data.get("registration");
            String kind = data.get("kind");
            String item = data.get("id");
            String factory = data.get("factory");
            if (!permitted(context) || !accepts(context, registration)
                || !("question".equals(kind) || "problem".equals(kind)) || item == null
                || !item.matches("[0-9a-fA-F-]{36}") || factory == null || factory.trim().isEmpty()) return;
            NotificationManager manager = context.getSystemService(NotificationManager.class);
            manager.createNotificationChannel(new NotificationChannel(CHANNEL, "Inbox", NotificationManager.IMPORTANCE_DEFAULT));
            Intent intent = new Intent(context, MainActivity.class).setFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .setData(Uri.parse("yoloforge://inbox/" + registration + "/" + kind + "/" + item));
            PendingIntent open = PendingIntent.getActivity(context, 0, intent, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            Notification note = new Notification.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_notification).setContentTitle(factory)
                .setContentText("question".equals(kind) ? "Ein Agent wartet auf deine Antwort." : "Ein Intent braucht deine Aufmerksamkeit.")
                .setVisibility(Notification.VISIBILITY_PRIVATE).setOnlyAlertOnce(true)
                .setAutoCancel(true).setContentIntent(open).build();
            manager.notify(PREFIX + kind + "/" + item, 1, note);
        }
    }
}
