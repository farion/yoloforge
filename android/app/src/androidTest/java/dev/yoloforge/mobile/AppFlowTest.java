package dev.yoloforge.mobile;

import android.app.Activity;
import android.app.NotificationManager;
import android.content.Intent;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.EditText;
import android.widget.TextView;
import android.widget.Spinner;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;
import static org.junit.Assume.assumeNotNull;
import org.json.JSONObject;

/** Real Rust-server acceptance supplied by scripts/smoke-android.py; no model calls. */
@RunWith(AndroidJUnit4.class)
public final class AppFlowTest {
    private Activity activity;
    private final android.app.Instrumentation instrument = InstrumentationRegistry.getInstrumentation();
    private View find(View view, String text, boolean edit) {
        if(edit && view instanceof EditText && text.contentEquals(((EditText)view).getHint()))return view;
        if(!edit && view instanceof Button && ((Button)view).getText().toString().startsWith(text))return view;
        if(view instanceof ViewGroup){ViewGroup group=(ViewGroup)view;for(int i=0;i<group.getChildCount();i++){View found=find(group.getChildAt(i),text,edit);if(found!=null)return found;}}
        return null;
    }
    private View waitView(String text, boolean edit)throws Exception {
        long deadline=System.nanoTime()+20_000_000_000L;
        while(System.nanoTime()<deadline){View[] found={null};instrument.runOnMainSync(()->found[0]=find(activity.getWindow().getDecorView(),text,edit));if(found[0]!=null)return found[0];Thread.sleep(50);}
        throw new AssertionError("Missing UI control: "+text);
    }
    private void click(String label)throws Exception {View view=waitView(label,false);instrument.runOnMainSync(()->view.performClick());}
    private void fill(String hint,String text)throws Exception {EditText view=(EditText)waitView(hint,true);instrument.runOnMainSync(()->view.setText(text));}
    private Spinner spinner(View view){if(view instanceof Spinner)return (Spinner)view;if(view instanceof ViewGroup){ViewGroup group=(ViewGroup)view;for(int i=0;i<group.getChildCount();i++){Spinner found=spinner(group.getChildAt(i));if(found!=null)return found;}}return null;}
    @Test public void enrollmentNotificationQuestionAndPromptAgainstRustServer()throws Exception {
        var arguments=InstrumentationRegistry.getArguments();String endpoint=arguments.getString("factoryEndpoint"),code=arguments.getString("enrollmentCode");assumeNotNull(endpoint,code);
        var context=instrument.getTargetContext();Vault vault=new Vault(context);vault.put("credentials",null);vault.put("pending",null);
        activity=instrument.startActivitySync(new Intent(context,MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        try {
            fill("Factory-Adresse (HTTPS)",endpoint);fill("Einmaliger Anmeldecode",code);click("Anmelden");
            click("Benachrichtigungen einschalten");
            NotificationManager notifications=context.getSystemService(NotificationManager.class);
            long deadline=System.nanoTime()+20_000_000_000L;boolean questionNotice=false;
            while(System.nanoTime()<deadline){for(var notification:notifications.getActiveNotifications())if(notification.getTag()!=null)questionNotice=true;if(questionNotice)break;Thread.sleep(100);}
            assertTrue("SSE service must publish the pending question from its initial snapshot",questionNotice);
            click("Android acceptance");click("Rueckfragen");click("Name eingeben");fill("name","Android UI proof");click("Antwort senden");
            click("Neuer Prompt");fill("Was soll bearbeitet werden?","Prompt from Android UI");
            instrument.runOnMainSync(()->spinner(activity.getWindow().getDecorView()).setSelection(1));
            click("Zurueck");click("Neuer Prompt");
            EditText restored=(EditText)waitView("Was soll bearbeitet werden?",true);
            instrument.runOnMainSync(()->{assertEquals("Prompt from Android UI",restored.getText().toString());assertEquals(1,spinner(activity.getWindow().getDecorView()).getSelectedItemPosition());});
            android.app.Instrumentation.ActivityMonitor monitor=instrument.addMonitor(MainActivity.class.getName(),null,false);
            instrument.runOnMainSync(()->activity.recreate());
            Activity replacement=instrument.waitForMonitorWithTimeout(monitor,10000);instrument.removeMonitor(monitor);
            assertNotNull("Activity must be recreated",replacement);activity=replacement;
            EditText rotated=(EditText)waitView("Was soll bearbeitet werden?",true);
            instrument.runOnMainSync(()->{assertEquals("Prompt from Android UI",rotated.getText().toString());assertEquals(1,spinner(activity.getWindow().getDecorView()).getSelectedItemPosition());});
            click("Senden");
            waitView("Neuer Prompt",false);
            JSONObject credential=vault.get("credentials");Api api=new Api(credential.getString("base"),credential.getString("token"));JSONObject snapshot=api.request("GET","/v1/snapshot",null);
            boolean answered=false,created=false;
            for(int i=0;i<snapshot.getJSONArray("questions").length();i++){JSONObject q=snapshot.getJSONArray("questions").getJSONObject(i);if(q.optJSONObject("answer")!=null && "Android UI proof".equals(q.getJSONObject("answer").optString("name")))answered=true;}
            for(int i=0;i<snapshot.getJSONArray("tickets").length();i++){JSONObject t=snapshot.getJSONArray("tickets").getJSONObject(i);if("Prompt from Android UI".equals(t.optString("prompt"))){created=true;assertFalse("Fixed session must survive saved draft",t.isNull("requested_session"));}}
            assertTrue("Question answer must reach real Factory API",answered);assertTrue("Prompt must reach real Factory API",created);assertNull(vault.get("pending"));
            click("Session: Android UI acceptance");click("Vollstaendiges Archiv");
            waitView("Anhang speichern: proof.bin",false);click("Naechste Seite");waitView("Zum Anfang",false);click("Zum Anfang");waitView("Anhang speichern: proof.bin",false);
        } finally {context.stopService(new Intent(context,EventService.class));instrument.runOnMainSync(()->activity.finish());}
    }
}
