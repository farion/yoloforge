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
import static org.junit.Assume.assumeTrue;
import org.json.JSONObject;

/** Real Rust-server acceptance supplied by scripts/smoke-android.py; no model calls. */
@RunWith(AndroidJUnit4.class)
public final class AppFlowTest {
    @Test public void overviewShowsIntakeAndExpiresCompletedIntents() throws Exception {
        JSONObject ticket=new JSONObject().put("status","backlog");
        assertEquals(0,MainActivity.overviewStage(ticket,null,200_000));
        ticket.put("draft",true);
        assertEquals(-1,MainActivity.overviewStage(ticket,null,200_000));
        ticket.put("draft",false).put("status","done").put("updated_unix_ms",200_000);
        assertEquals(5,MainActivity.overviewStage(ticket,null,319_999));
        assertEquals(-1,MainActivity.overviewStage(ticket,null,320_000));
        ticket.put("status","cancelled");
        assertEquals(-1,MainActivity.overviewStage(ticket,null,200_000));
    }

    @Test public void intentLabelPrefersItsPublishedAgentResult() throws Exception {
        JSONObject task = new JSONObject().put("prompt", "Original prompt");
        JSONObject agent = new JSONObject().put("phase", "succeeded").put("summary", " Result summary \n");
        JSONObject command = new JSONObject().put("kind", "run_agent").put("state", "succeeded")
                .put("result", new JSONObject().put("agent", agent));
        assertEquals("Result summary", MainActivity.taskLabel(task, command));
        assertEquals("Original prompt", MainActivity.taskLabel(task, null));
        agent.put("summary", " \n ");
        assertEquals("Original prompt", MainActivity.taskLabel(task, command));
        agent.put("phase", "failed").put("summary", "Technical failure");
        command.put("state", "failed");
        assertEquals("Original prompt", MainActivity.taskLabel(task, command));
    }

    @Test public void nativePushOnlyDisplaysForTheCurrentFactoryLogin() throws Exception {
        var context=instrument.getTargetContext();
        assumeTrue(InboxPush.permitted(context));
        Vault vault=new Vault(context);
        JSONObject credentials=vault.get("credentials"), flag=vault.get("inbox-push-enabled"), binding=vault.get("inbox-push-binding");
        String registration=java.util.UUID.randomUUID().toString(), item=java.util.UUID.randomUUID().toString();
        var message=java.util.Map.of("registration",registration,"id",item,"kind","question","factory","Werkstatt");
        var notifications=context.getSystemService(NotificationManager.class);
        String tag="inbox-push:question/"+item;
        try {
            vault.put("credentials",new JSONObject().put("push_login","active"));
            vault.put("inbox-push-enabled",new JSONObject().put("login","active"));
            vault.put("inbox-push-binding",new JSONObject().put("login","active").put("id",registration));
            assertTrue(InboxPush.accepts(context,registration));
            InboxPush.show(context,message);
            var first=waitForNotification(notifications,tag,true);
            assertNotNull(first);
            assertEquals("Werkstatt",first.getNotification().extras.getCharSequence(android.app.Notification.EXTRA_TITLE));
            notifications.cancel(tag,1);
            assertNull(waitForNotification(notifications,tag,false));
            vault.put("credentials",new JSONObject().put("push_login","replacement"));
            assertFalse(InboxPush.accepts(context,registration));
            InboxPush.show(context,message);
            assertFalse(java.util.Arrays.stream(notifications.getActiveNotifications()).anyMatch(n->tag.equals(n.getTag())));
        } finally {
            notifications.cancel(tag,1);
            vault.put("credentials",credentials);vault.put("inbox-push-enabled",flag);vault.put("inbox-push-binding",binding);
        }
    }
    private static android.service.notification.StatusBarNotification waitForNotification(NotificationManager manager,String tag,boolean expectPresent){
        long deadline=android.os.SystemClock.elapsedRealtime()+3000;
        android.service.notification.StatusBarNotification notification;
        do {
            notification=java.util.Arrays.stream(manager.getActiveNotifications()).filter(n->tag.equals(n.getTag())).findFirst().orElse(null);
            if((notification!=null)==expectPresent)return notification;
            android.os.SystemClock.sleep(25);
        }while(android.os.SystemClock.elapsedRealtime()<deadline);
        return notification;
    }

    private Activity activity;
    private final android.app.Instrumentation instrument = InstrumentationRegistry.getInstrumentation();

    @Test public void actionButtonsKeepAHitAreaTallerThanTheirBackground() {
        instrument.runOnMainSync(()->{
            var context=instrument.getTargetContext();
            for(String label:new String[]{"Neuer Prompt","Inbox","Projekt pausieren"}){
                Button button=FactoryStyle.button(context,label,label.equals("Neuer Prompt"));
                int unspecified=View.MeasureSpec.makeMeasureSpec(0,View.MeasureSpec.UNSPECIFIED);
                button.measure(unspecified,unspecified);
                int width=button.getMeasuredWidth(), height=button.getMeasuredHeight();
                button.layout(0,0,width,height);
                var background=(android.graphics.drawable.InsetDrawable)button.getBackground();
                background.setBounds(0,0,width,height);
                var visible=background.getDrawable().getBounds();
                assertTrue(height>=FactoryStyle.dp(context,44));
                assertEquals(FactoryStyle.dp(context,6),visible.top);
                assertEquals(visible.top,height-visible.bottom);
                assertTrue((visible.height()-button.getLayout().getHeight())/2<button.getPaddingStart());
            }
        });
    }

    private View find(View view, String text, boolean edit) {
        if(!view.isShown())return null;
        if(edit && view instanceof EditText && text.contentEquals(((EditText)view).getHint()))return view;
        if(!edit && view.isClickable() && (text.equals(view.getContentDescription())
            || view instanceof TextView && !(view instanceof EditText) && text.contentEquals(((TextView)view).getText())
            || view instanceof Button && ((Button)view).getText().toString().startsWith(text)))return view;
        if(view instanceof ViewGroup){ViewGroup group=(ViewGroup)view;for(int i=0;i<group.getChildCount();i++){View found=find(group.getChildAt(i),text,edit);if(found!=null)return found;}}
        return null;
    }
    private View waitView(String text, boolean edit)throws Exception {
        long deadline=System.nanoTime()+20_000_000_000L;
        while(System.nanoTime()<deadline){View[] found={null};instrument.runOnMainSync(()->found[0]=find(activity.getWindow().getDecorView(),text,edit));if(found[0]!=null)return found[0];Thread.sleep(50);}
        throw new AssertionError("Missing UI control: "+text);
    }
    private void click(String label)throws Exception {View view=waitView(label,false);instrument.runOnMainSync(()->view.performClick());}
    private boolean hasLabel(View view,String label){
        if(!view.isShown())return false;
        if(label.equals(view.getContentDescription())||view instanceof TextView&&label.contentEquals(((TextView)view).getText()))return true;
        if(view instanceof ViewGroup){ViewGroup group=(ViewGroup)view;for(int i=0;i<group.getChildCount();i++)if(hasLabel(group.getChildAt(i),label))return true;}
        return false;
    }
    private void assertLabel(String label){
        instrument.runOnMainSync(()->assertTrue("Missing visible label: "+label,hasLabel(activity.getWindow().getDecorView(),label)));
    }
    private void waitLabel(String label)throws Exception{
        long deadline=System.nanoTime()+20_000_000_000L;
        while(System.nanoTime()<deadline){
            boolean[] found={false};
            instrument.runOnMainSync(()->found[0]=hasLabel(activity.getWindow().getDecorView(),label));
            if(found[0])return;
            Thread.sleep(50);
        }
        throw new AssertionError("Missing visible label: "+label);
    }
    private void fill(String hint,String text)throws Exception {EditText view=(EditText)waitView(hint,true);instrument.runOnMainSync(()->view.setText(text));}
    private Spinner spinner(View view){if(view instanceof Spinner)return (Spinner)view;if(view instanceof ViewGroup){ViewGroup group=(ViewGroup)view;for(int i=0;i<group.getChildCount();i++){Spinner found=spinner(group.getChildAt(i));if(found!=null)return found;}}return null;}
    private int screenY(View view){
        int[] position=new int[2];instrument.runOnMainSync(()->view.getLocationOnScreen(position));return position[1];
    }
    private android.view.accessibility.AccessibilityNodeInfo dialogNode(String label)throws Exception{
        var automation=instrument.getUiAutomation();
        long deadline=System.nanoTime()+5_000_000_000L;
        while(System.nanoTime()<deadline){
            var root=automation.getRootInActiveWindow();
            if(root!=null)for(var node:root.findAccessibilityNodeInfosByText(label)){
                if(label.equalsIgnoreCase(String.valueOf(node.getText()))||label.contentEquals(node.getContentDescription()==null?"":node.getContentDescription()))return node;
            }
            Thread.sleep(50);
        }
        throw new AssertionError("Missing dialog control: "+label);
    }
    @Test public void projectOverlayAndManagementAgainstRustServer()throws Exception{
        var arguments=InstrumentationRegistry.getArguments();String endpoint=arguments.getString("factoryEndpoint"),key=arguments.getString("factoryKey");assumeNotNull(endpoint,key);
        var context=instrument.getTargetContext();Vault vault=new Vault(context);vault.put("credentials",null);vault.put("pending",null);
        activity=instrument.startActivitySync(new Intent(context,MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        try{
            fill("Factory-Adresse (HTTPS)",endpoint);fill("Zugangsschlüssel",key);click("Anmelden");
            click("Navigation öffnen");
            View shortcut=waitView("Neuer Intent",false);
            int before=screenY(shortcut);
            click("Projekt auswählen");View option=waitView("Android acceptance",false);
            assertEquals(before,screenY(shortcut));
            assertTrue("Dropdown must overlap the navigation",screenY(option)+option.getHeight()>before);
            instrument.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK);
            instrument.runOnMainSync(()->assertFalse(option.isAttachedToWindow()));

            click("Projekt auswählen");click("Android acceptance");
            View pause=waitView("Projekt pausieren",false);
            instrument.runOnMainSync(pause::performClick);
            waitForProjectPause(endpoint,key,true);
            long deadline=System.nanoTime()+5_000_000_000L;
            while(pause.isAttachedToWindow()&&System.nanoTime()<deadline)Thread.sleep(50);
            assertFalse(pause.isAttachedToWindow());
            click("Navigation öffnen");click("Projekt fortsetzen");
            waitForProjectPause(endpoint,key,false);
        }finally{instrument.runOnMainSync(()->activity.finish());}
    }
    private void waitForProjectPause(String endpoint,String key,boolean paused)throws Exception{
        Api api=new Api(endpoint,key);
        long deadline=System.nanoTime()+5_000_000_000L;
        while(System.nanoTime()<deadline){
            if(api.request("GET","/v1/snapshot",null).getJSONArray("projects").getJSONObject(0).optBoolean("paused")==paused)return;
            Thread.sleep(50);
        }
        fail("Project pause state not updated");
    }
    @Test public void validKeySignsInAndInvalidKeyIsNotSaved()throws Exception {
        var arguments=InstrumentationRegistry.getArguments();String endpoint=arguments.getString("factoryEndpoint"),key=arguments.getString("factoryKey");assumeNotNull(endpoint,key);
        var context=instrument.getTargetContext();Vault vault=new Vault(context);vault.put("credentials",null);vault.put("pending",null);
        activity=instrument.startActivitySync(new Intent(context,MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        try {
            fill("Factory-Adresse (HTTPS)",endpoint);fill("Zugangsschlüssel","invalid-test-key");
            View signIn=waitView("Anmelden",false);
            instrument.runOnMainSync(()->{signIn.performClick();assertFalse(signIn.isEnabled());});
            long rejectedBy=System.nanoTime()+20_000_000_000L;
            boolean[] ready={false};
            while(!ready[0]&&System.nanoTime()<rejectedBy){
                instrument.runOnMainSync(()->ready[0]=signIn.isEnabled());
                if(!ready[0])Thread.sleep(50);
            }
            assertTrue("An invalid key must allow another attempt",ready[0]);
            assertNull("An invalid key must not be stored",vault.get("credentials"));
            fill("Zugangsschlüssel",key);click("Anmelden");
            waitView("Navigation öffnen",false);
            click("Navigation öffnen");click("Projekt auswählen");waitView("Android acceptance",false);
            assertTrue("The verified key must be stored in the vault",key.equals(vault.get("credentials").getString("token")));
        } finally {instrument.runOnMainSync(()->activity.finish());}
    }
    @Test public void accessKeyNotificationQuestionAndPromptAgainstRustServer()throws Exception {
        var arguments=InstrumentationRegistry.getArguments();String endpoint=arguments.getString("factoryEndpoint"),key=arguments.getString("factoryKey");assumeNotNull(endpoint,key);
        var context=instrument.getTargetContext();Vault vault=new Vault(context);vault.put("credentials",null);vault.put("pending",null);
        activity=instrument.startActivitySync(new Intent(context,MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        try {
            fill("Factory-Adresse (HTTPS)",endpoint);fill("Zugangsschlüssel",key);click("Anmelden");
            click("Navigation öffnen");assertLabel("Factory  ›  Overview");
            click("Projekt auswählen");assertLabel("Factory  ›  Overview");
            click("Android acceptance");assertLabel("Factory  ›  Overview");assertLabel("Projektlogo");
            click("Neuer Intent");waitView("Was soll bearbeitet werden?",true);click("Zurueck");
            click("Navigation öffnen");click("Sessions");waitView("Android UI acceptance",false);
            click("Navigation öffnen");click("Projektaktivität");assertLabel("Android acceptance  ›  Projektaktivität");
            click("Navigation öffnen");click("Infrastructure");waitView("Android fixture",false);
            click("Navigation öffnen");click("Diagnose");waitView("question.created",false);
            click("Navigation öffnen");click("Benachrichtigungen");waitView("Antwort erforderlich",false);
            click("Navigation öffnen");click("App-Einstellungen");click("Android-Benachrichtigungen");click("Benachrichtigungen einschalten");
            NotificationManager notifications=context.getSystemService(NotificationManager.class);
            long deadline=System.nanoTime()+20_000_000_000L;boolean questionNotice=false;
            while(System.nanoTime()<deadline){for(var notification:notifications.getActiveNotifications())if(notification.getTag()!=null)questionNotice=true;if(questionNotice)break;Thread.sleep(100);}
            assertTrue("SSE service must publish the pending question from its initial snapshot",questionNotice);
            click("Navigation öffnen");click("Inbox");assertLabel("Android acceptance  ›  Inbox");
            click("Archiv (0)");click("Offen (1)");
            fill("Inbox durchsuchen …","Name");click("Name eingeben");fill("name","Android UI proof");click("Antwort senden");
            waitLabel("Android acceptance  ›  Inbox");
            click("Navigation öffnen");click("Neuer Intent");fill("Was soll bearbeitet werden?","Prompt from Android UI");
            instrument.runOnMainSync(()->spinner(activity.getWindow().getDecorView()).setSelection(1));
            click("Zurueck");click("Navigation öffnen");click("Neuer Intent");
            EditText restored=(EditText)waitView("Was soll bearbeitet werden?",true);
            instrument.runOnMainSync(()->{assertEquals("Prompt from Android UI",restored.getText().toString());assertEquals(1,spinner(activity.getWindow().getDecorView()).getSelectedItemPosition());});
            android.app.Instrumentation.ActivityMonitor monitor=instrument.addMonitor(MainActivity.class.getName(),null,false);
            instrument.runOnMainSync(()->activity.recreate());
            Activity replacement=instrument.waitForMonitorWithTimeout(monitor,10000);instrument.removeMonitor(monitor);
            assertNotNull("Activity must be recreated",replacement);activity=replacement;
            EditText rotated=(EditText)waitView("Was soll bearbeitet werden?",true);
            instrument.runOnMainSync(()->{assertEquals("Prompt from Android UI",rotated.getText().toString());assertEquals(1,spinner(activity.getWindow().getDecorView()).getSelectedItemPosition());});
            click("Senden");
            waitView("Queue",false);
            JSONObject credential=vault.get("credentials");Api api=new Api(credential.getString("base"),credential.getString("token"));JSONObject snapshot=api.request("GET","/v1/snapshot",null);
            boolean answered=false,created=false;
            for(int i=0;i<snapshot.getJSONArray("questions").length();i++){JSONObject q=snapshot.getJSONArray("questions").getJSONObject(i);if(q.optJSONObject("answer")!=null && "Android UI proof".equals(q.getJSONObject("answer").optString("name")))answered=true;}
            for(int i=0;i<snapshot.getJSONArray("tickets").length();i++){JSONObject t=snapshot.getJSONArray("tickets").getJSONObject(i);if("Prompt from Android UI".equals(t.optString("prompt"))){created=true;assertFalse("Fixed session must survive saved draft",t.isNull("requested_session"));}}
            assertTrue("Question answer must reach real Factory API",answered);assertTrue("Prompt must reach real Factory API",created);assertNull(vault.get("pending"));
            click("Navigation öffnen");click("Sessions");click("Android UI acceptance");click("Vollstaendiges Archiv");
            waitView("Anhang speichern: proof.bin",false);click("Naechste Seite");waitView("Zum Anfang",false);click("Zum Anfang");waitView("Anhang speichern: proof.bin",false);
        } finally {context.stopService(new Intent(context,EventService.class));instrument.runOnMainSync(()->activity.finish());}
    }
}
