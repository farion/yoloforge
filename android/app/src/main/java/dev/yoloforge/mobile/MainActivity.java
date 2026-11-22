package dev.yoloforge.mobile;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.os.Bundle;
import android.text.InputType;
import android.util.Base64;
import android.view.View;
import android.widget.*;
import java.util.*;
import java.util.concurrent.Executors;
import org.json.*;

public final class MainActivity extends Activity {
    private final java.util.concurrent.ExecutorService worker = Executors.newSingleThreadExecutor();
    private Vault vault;
    private LinearLayout page;
    private TextView status;
    private JSONObject snapshot;
    private String projectId;
    private final ArrayList<JSONObject> images = new ArrayList<>();
    private EditText prompt;
    private TextView imageLabel;
    private String deepQuestion;
    private int generation;

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved); vault = new Vault(this); deepQuestion = getIntent().getStringExtra("question");
        try { if (vault.get("credentials") == null) login(); else refresh(); } catch (Exception error) { login(); message(error.getMessage()); }
    }
    @Override public void onNewIntent(Intent intent) { super.onNewIntent(intent); deepQuestion = intent.getStringExtra("question"); refresh(); }
    @Override public void onDestroy() { worker.shutdown(); super.onDestroy(); }
    private void screen(String title) {
        generation++;
        LinearLayout root = new LinearLayout(this); root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(24,24,24,24);
        root.setOnApplyWindowInsetsListener((v,insets)->{v.setPadding(24+insets.getSystemWindowInsetLeft(),24+insets.getSystemWindowInsetTop(),24+insets.getSystemWindowInsetRight(),24+insets.getSystemWindowInsetBottom());return insets;});
        setContentView(root); TextView heading = new TextView(this); heading.setText(getString(R.string.page_title,title)); heading.setTextSize(23); root.addView(heading);
        status = new TextView(this); status.setTextSize(14); root.addView(status);
        ScrollView scroll = new ScrollView(this); root.addView(scroll,new LinearLayout.LayoutParams(-1,0,1));
        page = new LinearLayout(this); page.setOrientation(LinearLayout.VERTICAL); scroll.addView(page);
    }
    private void text(String value) { TextView view = new TextView(this); view.setText(value); view.setTextIsSelectable(true); view.setPadding(0,10,0,10); page.addView(view); }
    private void button(String label, Runnable action) { Button button = new Button(this); button.setText(label); button.setOnClickListener(v->action.run()); page.addView(button); }
    private EditText input(String hint, String value, int type) { EditText edit = new EditText(this); edit.setHint(hint); edit.setContentDescription(hint); edit.setText(value); edit.setInputType(type); page.addView(edit); return edit; }
    private void message(String value) { if (status != null) status.setText(value == null ? "Verbindung fehlgeschlagen" : value); }
    private interface Work { void run() throws Exception; }
    private void background(Work work) { int started=generation;worker.execute(()->{try {work.run();} catch(Exception e) {ui(()->{if(generation==started)message(e.getMessage());});}}); }
    private void ui(Runnable action) { runOnUiThread(()->{if(!isDestroyed()) action.run();}); }
    private Api api() throws Exception { JSONObject c = vault.get("credentials"); if(c==null) throw new Exception("Bitte anmelden"); return new Api(c.getString("base"),c.getString("token")); }
    private JSONArray array(String key) { return snapshot == null ? new JSONArray() : snapshot.optJSONArray(key) == null ? new JSONArray() : snapshot.optJSONArray(key); }
    private JSONObject find(String key,String id) { JSONArray values=array(key); for(int i=0;i<values.length();i++){JSONObject v=values.optJSONObject(i);if(v!=null && v.optString("id").equals(id))return v;} return null; }
    private void login() {
        screen("Anmeldung"); text("In der Desktop-TUI unter Geraete einen einmaligen Operator-Anmeldecode erstellen.");
        EditText endpoint=input("Factory-Adresse (HTTPS)","",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_URI);
        EditText code=input("Einmaliger Anmeldecode","",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_PASSWORD);
        EditText name=input("Geraetename","Android",InputType.TYPE_CLASS_TEXT);
        button("Anmelden",()->{
            String base=endpoint.getText().toString(), secret=code.getText().toString(), label=name.getText().toString();
            code.setText(""); message("Melde Geraet an …");
            background(()->{
                Api enrollment=new Api(base,null); JSONObject pending=vault.get("pending");
                if(pending!=null && !pending.getString("base").equals(enrollment.base))throw new Exception("Ungeklaerter Auftrag gehoert zu einer anderen Factory");
                JSONObject result=enrollment.request("POST","/v1/enroll",new JSONObject().put("code",secret).put("name",label));
                vault.put("credentials",new JSONObject().put("base",enrollment.base).put("token",result.getString("token")));
                ui(this::refresh);
            });
        });
        text("Bei unklarer Anmeldung einen neuen Code erzeugen; der Einmalcode wird nicht automatisch erneut gesendet.");
    }
    private void refresh() {
        screen("Verbinden"); message("Lade Projekte und Sessions …");
        int started=generation;
        background(()->{JSONObject fresh=api().request("GET","/v1/snapshot",null); JSONObject pending=vault.get("pending"); ui(()->{
            if(generation!=started)return;
            snapshot=fresh;
            if(pending!=null){pendingScreen();return;}
            if(deepQuestion!=null){JSONObject q=find("questions",deepQuestion);deepQuestion=null;if(q!=null){answer(q);return;}}
            if(projectId!=null && find("projects",projectId)!=null) project(); else projects();
        });});
        button("Neu anmelden",this::login);
    }
    private void projects() {
        screen("Projekte");button("Aktualisieren",this::refresh);button("Rueckfragen",this::questions);
        button("Benachrichtigungen einschalten",()->{
            if(android.os.Build.VERSION.SDK_INT>=33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=android.content.pm.PackageManager.PERMISSION_GRANTED)
                requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},10);
            else startForegroundService(new Intent(this,EventService.class));
        });
        button("Benachrichtigungen ausschalten",()->stopService(new Intent(this,EventService.class)));
        JSONArray projects=array("projects"); for(int i=0;i<projects.length();i++){JSONObject p=projects.optJSONObject(i);if(p==null)continue;
            button(p.optString("name")+(p.optBoolean("paused")?" · pausiert":""),()->{projectId=p.optString("id");project();});}
        if(projects.length()==0)text("Noch keine Projekte. Projekt auf dem Desktop mit dem gewuenschten Runner anlegen.");
        button("Anmeldung wechseln",this::login);
    }
    @Override public void onRequestPermissionsResult(int request,String[] permissions,int[] results){super.onRequestPermissionsResult(request,permissions,results);if(request==10 && results.length>0 && results[0]==android.content.pm.PackageManager.PERMISSION_GRANTED)startForegroundService(new Intent(this,EventService.class));}
    private void project() {
        JSONObject p=find("projects",projectId); if(p==null){projects();return;}
        screen(p.optString("name"));button("Projekte",()->{projectId=null;projects();});button("Aktualisieren",this::refresh);
        button("Neuer Prompt",this::compose);button("Rueckfragen",this::questions);
        button(p.optBoolean("paused")?"Projekt fortsetzen":"Projekt pausieren",()->{
            try{write("/v1/projects/"+projectId+"/pause",new JSONObject().put("request_id",UUID.randomUUID().toString()).put("expected_revision",p.getLong("revision")).put("paused",!p.optBoolean("paused")));}catch(Exception e){message(e.getMessage());}
        });
        text("Maximal arbeitende Sessions: "+p.optInt("max_sessions",1));
        JSONArray tasks=array("tickets");for(int i=0;i<tasks.length();i++){JSONObject t=tasks.optJSONObject(i);if(t==null || !t.optString("project_id").equals(projectId))continue;
            JSONObject c=find("commands",t.optString("execution_command"));String detail=c==null?t.optString("routing_note","Wartet"):c.optString("state");
            text(t.optString("prompt",t.optString("title"))+"\n"+detail);
        }
        JSONArray streams=array("streams");for(int i=0;i<streams.length();i++){JSONObject s=streams.optJSONObject(i);if(s!=null && s.optString("project_id").equals(projectId))button("Session: "+s.optString("goal")+" · "+s.optString("status"),()->session(s));}
    }
    private void compose() {
        images.clear(); screen("Prompt");button("Zurueck",this::project);
        prompt=input("Was soll bearbeitet werden?","",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_FLAG_MULTI_LINE);prompt.setMinLines(5);
        ArrayList<String> labels=new ArrayList<>(),roots=new ArrayList<>();labels.add("Orchestrator entscheidet");roots.add(null);
        HashSet<String> seen=new HashSet<>();JSONArray commands=array("commands");
        for(int i=commands.length()-1;i>=0;i--){JSONObject c=commands.optJSONObject(i);if(c==null || !c.optString("kind").equals("run_agent"))continue;
            JSONObject spec=c.optJSONObject("agent_spec"),s=find("streams",c.optString("stream_id"));if(spec==null || spec.optBoolean("read_only") || s==null || !s.optString("project_id").equals(projectId))continue;
            JSONObject continuation=spec.optJSONObject("continuation");String root=continuation==null?c.optString("id"):continuation.optString("root_command_id");
            if(seen.add(root)){labels.add(s.optString("goal")+" · "+root.substring(0,8));roots.add(root);}
        }
        Spinner sessions=new Spinner(this);sessions.setContentDescription("Feste Session oder Orchestrator");sessions.setAdapter(new ArrayAdapter<>(this,android.R.layout.simple_spinner_dropdown_item,labels));page.addView(sessions);
        imageLabel=new TextView(this);page.addView(imageLabel);
        button("Bild anhaengen",()->{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE);startActivityForResult(pick,11);});
        button("Bilder entfernen",()->{images.clear();imageLabel.setText("");});
        button("Senden",()->{try{
            String text=prompt.getText().toString();if(text.trim().isEmpty() && images.isEmpty())throw new Exception("Prompt oder Bild eingeben");
            JSONObject body=new JSONObject().put("request_id",UUID.randomUUID().toString()).put("id",JSONObject.NULL).put("expected_revision",0).put("project_id",projectId).put("prompt",text).put("images",new JSONArray(images));
            String root=roots.get(sessions.getSelectedItemPosition());body.put("requested_session",root==null?JSONObject.NULL:root);write("/v1/tasks",body);
        }catch(Exception e){message(e.getMessage());}});
    }
    @Override public void onActivityResult(int request,int result,Intent data){super.onActivityResult(request,result,data);if(request!=11 || result!=RESULT_OK || data==null || data.getData()==null)return;
        try{
            if(images.size()>=4)throw new Exception("Maximal vier Bilder");String mime=getContentResolver().getType(data.getData());
            if(!Arrays.asList("image/png","image/jpeg","image/gif","image/webp").contains(mime))throw new Exception("PNG, JPEG, GIF oder WebP verwenden");
            byte[] bytes;try(var in=getContentResolver().openInputStream(data.getData())){bytes=Api.read(in,4*1024*1024);}
            int total=bytes.length;for(JSONObject image:images)total+=Base64.decode(image.getString("data"),Base64.NO_WRAP).length;
            if(total>4*1024*1024)throw new Exception("Bilder zusammen maximal 4 MiB");
            images.add(new JSONObject().put("name","image-"+(images.size()+1)).put("mime",mime).put("data",Base64.encodeToString(bytes,Base64.NO_WRAP)));
            imageLabel.setText(getString(R.string.attached_images,images.size()));
        }catch(Exception e){message(e.getMessage());}
    }
    private void session(JSONObject stream) {
        screen("Session");button("Zurueck",this::project);JSONObject latest=null;JSONArray commands=array("commands");for(int i=commands.length()-1;i>=0;i--){JSONObject c=commands.optJSONObject(i);if(c!=null && c.optString("stream_id").equals(stream.optString("id")) && c.optString("kind").equals("run_agent")){latest=c;break;}}
        if(latest==null){text("Session wird vorbereitet");return;}
        text(latest.optString("state"));JSONObject result=latest.optJSONObject("result");if(result!=null)text(result.toString());
        String id=latest.optString("id");int started=generation;background(()->{JSONObject excerpt=api().request("GET","/v1/commands/"+id+"/excerpt",null);String rendered=excerpt.toString(2);ui(()->{if(generation==started)text(rendered);});});
    }
    private void questions() {
        screen("Rueckfragen");button("Projekte",this::projects);button("Aktualisieren",()->{deepQuestion=null;projectId=null;refresh();});
        JSONArray questions=array("questions");int count=0;for(int i=0;i<questions.length();i++){JSONObject q=questions.optJSONObject(i);if(q==null || !Arrays.asList("pending","rejected").contains(q.optString("state")))continue;count++;
            JSONObject ext=q.optJSONObject("external"),payload=ext==null?null:ext.optJSONObject("payload");button(payload==null?"Rueckfrage":payload.optString("title",payload.optString("action","Rueckfrage")),()->answer(q));}
        if(count==0)text("Keine offenen Rueckfragen");
    }
    private void answer(JSONObject q) {
        screen("Antwort");button("Zurueck",this::questions);text(q.optString("orchestration_note"));
        JSONObject ext=q.optJSONObject("external"),payload=ext==null?null:ext.optJSONObject("payload");if(payload==null)return;
        text(payload.optString("title",payload.optString("action")));text(payload.optString("description"));
        if("permission".equals(ext.optString("kind"))){text(payload.optJSONArray("resources")==null?"":payload.optJSONArray("resources").toString());
            button("Einmal erlauben",()->permission(q,"once"));button("Ablehnen",()->permission(q,"reject"));return;}
        JSONArray fields=payload.optJSONArray("fields");if(fields==null)return;
        LinkedHashMap<String,View> inputs=new LinkedHashMap<>();
        for(int i=0;i<fields.length();i++){JSONObject f=fields.optJSONObject(i);if(f==null)continue;
            if(f.has("when") && f.optJSONArray("when")!=null && f.optJSONArray("when").length()>0 || f.optString("type").equals("external")){text("Dieses Formular bitte in OpenCode beantworten (Bedingungen/Anmeldung).");return;}
            String key=f.optString("key"),type=f.optString("type");text(f.optString("title",key)+(f.optBoolean("required")?" *":""));
            if(type.equals("boolean")){CheckBox box=new CheckBox(this);box.setChecked(f.optBoolean("default"));page.addView(box);inputs.put(key,box);}
            else if(type.equals("string") || type.equals("integer") || type.equals("number")) {
                JSONArray options=f.optJSONArray("options");if(options!=null && !f.optBoolean("custom")){ArrayList<String> values=new ArrayList<>();for(int j=0;j<options.length();j++)values.add(options.optJSONObject(j).optString("value"));Spinner choice=new Spinner(this);choice.setAdapter(new ArrayAdapter<>(this,android.R.layout.simple_spinner_dropdown_item,values));page.addView(choice);inputs.put(key,choice);}
                else inputs.put(key,input(key,f.optString("default",""),InputType.TYPE_CLASS_TEXT));
            } else {text("Dieses Feld bitte in der Desktop-TUI beantworten: "+type);return;}
        }
        button("Antwort senden",()->{try{JSONObject answer=new JSONObject();for(int i=0;i<fields.length();i++){JSONObject f=fields.getJSONObject(i);String key=f.getString("key");View view=inputs.get(key);Object value;
            if(view instanceof CheckBox)value=((CheckBox)view).isChecked();else if(view instanceof Spinner)value=((Spinner)view).getSelectedItem();else {String raw=((EditText)view).getText().toString();if(raw.isEmpty()&&!f.optBoolean("required"))continue;if(raw.isEmpty())throw new Exception("Pflichtfeld fehlt: "+key);value=f.optString("type").equals("integer")?Long.valueOf(raw):f.optString("type").equals("number")?Double.valueOf(raw):raw;}
            answer.put(key,value);
        }sendAnswer(q,answer);}catch(Exception e){message(e.getMessage());}});
    }
    private void permission(JSONObject q,String decision){try{sendAnswer(q,new JSONObject().put("decision",decision));}catch(Exception e){message(e.getMessage());}}
    private void sendAnswer(JSONObject q,JSONObject answer)throws Exception{write("/v1/questions/"+q.getString("id")+"/answer",new JSONObject().put("request_id",UUID.randomUUID().toString()).put("expected_revision",q.getLong("revision")).put("answer",answer));}
    private void write(String path,JSONObject body)throws Exception {
        if(vault.get("pending")!=null){pendingScreen();return;}
        new Outbox(vault).prepare(api(),path,body);pendingScreen();retry();
    }
    private void pendingScreen(){screen("Auftrag gespeichert");text("Bei Verbindungsabbruch bleibt der identische Auftrag mit derselben Request-ID gespeichert. Erneutes Senden erzeugt keinen zweiten Auftrag.");button("Erneut senden",this::retry);button("Neu anmelden",this::login);}
    private void retry(){message("Sende gespeicherten Auftrag …");background(()->{
        JSONObject pending=vault.get("pending");if(pending==null){ui(this::refresh);return;}Api client=api();if(!client.base.equals(pending.getString("base")))throw new Exception("Auftrag gehoert zu einer anderen Factory");
        try{new Outbox(vault).send(client);ui(this::refresh);}
        catch(Api.HttpFailure e){if(Arrays.asList(400,403,404,409,422).contains(e.status)){ui(()->{projects();message("Auftrag abgelehnt (HTTP "+e.status+"). Bitte aktualisieren und korrigieren.");});}else throw e;}
    });}
}
