package dev.yoloforge.mobile;

import android.app.*;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.IBinder;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.HttpURLConnection;
import java.nio.charset.StandardCharsets;
import org.json.*;

/** Direct Factory SSE notifications; no third-party push provider or credential relay. */
public final class EventService extends Service {
    private volatile boolean stopped;
    private volatile HttpURLConnection connection;
    private Thread thread;
    private NotificationManager notifications;
    @Override public void onCreate() {
        super.onCreate();notifications=getSystemService(NotificationManager.class);
        notifications.createNotificationChannel(new NotificationChannel("connection","Factory-Verbindung",NotificationManager.IMPORTANCE_LOW));
        notifications.createNotificationChannel(new NotificationChannel("questions","Rueckfragen",NotificationManager.IMPORTANCE_DEFAULT));
        Notification running=new Notification.Builder(this,"connection").setSmallIcon(android.R.drawable.stat_notify_sync).setContentTitle("YoloForge: Liveverbindung")
            .setContentText("Verbindungsaufbau und Empfang; in der App abschaltbar").setOngoing(true).setContentIntent(open(null)).build();
        if(android.os.Build.VERSION.SDK_INT>=29)startForeground(1,running,ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);else startForeground(1,running);
        thread=new Thread(this::listen,"yoloforge-events");thread.start();
    }
    @Override public int onStartCommand(Intent intent,int flags,int startId){return START_NOT_STICKY;}
    @Override public IBinder onBind(Intent intent){return null;}
    @Override public void onDestroy(){stopped=true;if(connection!=null)connection.disconnect();if(thread!=null)thread.interrupt();super.onDestroy();}
    @Override public void onTimeout(int startId,int fgsType){stopForeground(STOP_FOREGROUND_REMOVE);stopSelf();}
    private PendingIntent open(String question){Intent intent=new Intent(this,MainActivity.class).setFlags(Intent.FLAG_ACTIVITY_NEW_TASK|Intent.FLAG_ACTIVITY_SINGLE_TOP);
        if(question!=null){intent.putExtra("question",question);intent.setData(android.net.Uri.parse("yoloforge://question/"+question));}
        return PendingIntent.getActivity(this,0,intent,PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
    }
    private void question(JSONObject q)throws Exception {
        String id=q.getString("id");long revision=q.getLong("revision");var prefs=getSharedPreferences("events",MODE_PRIVATE);
        if(!"pending".equals(q.optString("state")) && !"rejected".equals(q.optString("state"))){notifications.cancel(id,2);return;}
        if(prefs.getLong("question:"+id,0)>=revision)return;
        Notification note=new Notification.Builder(this,"questions").setSmallIcon(android.R.drawable.ic_dialog_info).setContentTitle("YoloForge: Rueckfrage")
            .setContentText("Eine Session braucht eine Antwort.").setVisibility(Notification.VISIBILITY_PRIVATE).setOnlyAlertOnce(true).setAutoCancel(true).setContentIntent(open(id)).build();
        if(android.os.Build.VERSION.SDK_INT<33 || checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)==android.content.pm.PackageManager.PERMISSION_GRANTED){notifications.notify(id,2,note);persist(prefs.edit().putLong("question:"+id,revision));}
    }
    private static void persist(android.content.SharedPreferences.Editor edit)throws Exception{if(!edit.commit())throw new Exception("Eventposition nicht gespeichert");}
    private void listen(){long delay=1000;Vault vault=new Vault(this);
        while(!stopped){
            try {
                JSONObject credential=vault.get("credentials");if(credential==null){stopSelf();return;}
                Api api=new Api(credential.getString("base"),credential.getString("token"));var prefs=getSharedPreferences("events",MODE_PRIVATE);
                if(!api.base.equals(prefs.getString("origin","")))persist(prefs.edit().clear().putString("origin",api.base));
                JSONObject snapshot=api.request("GET","/v1/snapshot",null);JSONArray questions=snapshot.optJSONArray("questions");
                if(questions!=null)for(int i=0;i<questions.length();i++)question(questions.getJSONObject(i));
                // Snapshot repairs gaps after disconnect; start streaming after that exact snapshot.
                long cursor=snapshot.getLong("event_cursor");persist(prefs.edit().putLong("cursor",cursor));
                connection=api.open("/v1/events/stream");connection.setReadTimeout(60000);connection.setRequestProperty("last-event-id",Long.toString(cursor));
                int status=connection.getResponseCode();if(status==401||status==403){stopSelf();return;}if(status!=200)throw new Exception("Eventstream nicht erreichbar");
                delay=1000;
                try(BufferedReader reader=new BufferedReader(new InputStreamReader(connection.getInputStream(),StandardCharsets.UTF_8))){
                    StringBuilder data=new StringBuilder();String line;
                    while(!stopped && (line=reader.readLine())!=null){
                        if(line.isEmpty()){
                            if(data.length()>0){JSONObject event=new JSONObject(data.toString());long sequence=event.getLong("sequence");
                                if(sequence>cursor){String kind=event.optString("kind");if(kind.equals("question.created")||kind.equals("question.updated"))question(event.getJSONObject("payload"));cursor=sequence;persist(prefs.edit().putLong("cursor",cursor));}
                            }data.setLength(0);
                        }else if(line.startsWith("data:")){if(data.length()>0)data.append('\n');data.append(line.startsWith("data: ")?line.substring(6):line.substring(5));if(data.length()>8*1024*1024)throw new Exception("Event zu gross");}
                    }
                }
            }catch(Api.HttpFailure e){if(e.status==401||e.status==403){stopSelf();return;}}
            catch(Exception ignored){/* No prompts, tokens or provider errors in Android logs. */}
            finally {if(connection!=null){connection.disconnect();connection=null;}}
            if(!stopped)try{Thread.sleep(delay);}catch(InterruptedException ignored){return;}
            delay=Math.min(30000,delay*2);
        }
    }
}
