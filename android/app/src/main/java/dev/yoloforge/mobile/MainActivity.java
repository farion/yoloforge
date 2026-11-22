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
    private boolean composing;
    private Spinner composeSessions;
    private ArrayList<String> composeRoots;
    private boolean restoreCompose;
    private String restoreBase;
    private String archiveCommand;
    private long archiveAfter;
    private Long archiveRevision;
    private android.net.Uri pickedImage;
    private FormEditor form;
    private JSONObject editingQuestion;
    private String restoreQuestion;
    private boolean showQuestions;
    private void readIntent(Intent intent){deepQuestion=intent.getStringExtra("question");android.net.Uri uri=intent.getData();showQuestions=uri!=null && "yoloforge".equals(uri.getScheme()) && "questions".equals(uri.getHost());}
    private String answerDraftKey(JSONObject q)throws Exception{return "answer-draft:"+api().base+":"+q.getString("id")+":"+q.getLong("revision");}
    private void saveAnswerDraft()throws Exception {if(form!=null && editingQuestion!=null)vault.put(answerDraftKey(editingQuestion),form.draft());}

    private String draftKey()throws Exception{return "draft:"+api().base+":"+projectId;}
    private void saveDraft()throws Exception {
        if(!composing || prompt==null || composeSessions==null)return;
        String root=composeRoots.get(composeSessions.getSelectedItemPosition());
        vault.put(draftKey(),new JSONObject().put("prompt",prompt.getText().toString()).put("images",new JSONArray(images)).put("requested_session",root==null?JSONObject.NULL:root));
    }
    @Override public void onPause(){try{saveDraft();saveAnswerDraft();}catch(Exception e){message("Entwurf konnte nicht gespeichert werden");}super.onPause();}
    @Override public void onSaveInstanceState(Bundle state) {
        try {
            saveDraft();
            saveAnswerDraft();
            state.putString("answer",editingQuestion==null?restoreQuestion:editingQuestion.optString("id"));
            state.putString("factory",api().base);
            state.putString("project",projectId);
            state.putBoolean("compose",composing || restoreCompose);
            state.putString("archive",archiveCommand);
            state.putLong("archiveAfter",archiveAfter);
            if(archiveRevision!=null)state.putLong("archiveRevision",archiveRevision);
        }catch(Exception e){message("Ansicht konnte nicht gespeichert werden");}
        super.onSaveInstanceState(state);
    }

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved); vault = new Vault(this);readIntent(getIntent());
        if(saved!=null){restoreBase=saved.getString("factory");projectId=saved.getString("project");restoreCompose=saved.getBoolean("compose");restoreQuestion=saved.getString("answer");archiveCommand=saved.getString("archive");archiveAfter=saved.getLong("archiveAfter");if(saved.containsKey("archiveRevision"))archiveRevision=saved.getLong("archiveRevision");}
        try { if (vault.get("credentials") == null) login(); else refresh(); } catch (Exception error) { login(); message(error.getMessage()); }
    }
    @Override public void onNewIntent(Intent intent) { super.onNewIntent(intent);setIntent(intent);readIntent(intent);refresh(); }
    @Override public void onDestroy() { worker.shutdown(); super.onDestroy(); }
    private void screen(String title) {
        try{saveDraft();saveAnswerDraft();}catch(Exception e){android.widget.Toast.makeText(this,"Entwurf konnte nicht gespeichert werden",android.widget.Toast.LENGTH_LONG).show();}
        form=null;editingQuestion=null;
        composing=false;
        if(!title.equals("Archiv") && !title.equals("Verbinden"))archiveCommand=null;
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
            if(showQuestions){showQuestions=false;questions();return;}
            try{if(restoreBase!=null && !restoreBase.equals(api().base)){projectId=null;restoreCompose=false;archiveCommand=null;restoreQuestion=null;}}catch(Exception e){return;}restoreBase=null;
            if(deepQuestion!=null){JSONObject q=find("questions",deepQuestion);deepQuestion=null;if(q!=null){answer(q);return;}}
            if(restoreQuestion!=null){JSONObject q=find("questions",restoreQuestion);restoreQuestion=null;if(q!=null){answer(q);return;}}
            if(restoreCompose && projectId!=null && find("projects",projectId)!=null){restoreCompose=false;compose();return;}
            restoreCompose=false;
            if(archiveCommand!=null){archive(archiveCommand,archiveAfter,archiveRevision);return;}
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
        button("Hintergrund-Push (ntfy)",this::pushSettings);
        button("App-Updates",this::updates);
        JSONArray projects=array("projects"); for(int i=0;i<projects.length();i++){JSONObject p=projects.optJSONObject(i);if(p==null)continue;
            button(p.optString("name")+(p.optBoolean("paused")?" · pausiert":""),()->{projectId=p.optString("id");project();});}
        if(projects.length()==0)text("Noch keine Projekte. Projekt auf dem Desktop mit dem gewuenschten Runner anlegen.");
        button("Anmeldung wechseln",this::login);
    }
    private void pushSettings(){
        screen("Hintergrund-Push");button("Zurueck",this::projects);
        text("ntfy empfaengt Hinweise auch bei geschlossener YoloForge-App. Factory benoetigt --push-config mit derselben Topic-Adresse. Die Nachricht enthaelt nur einen allgemeinen Hinweis; Antworten werden erst nach Anmeldung direkt aus Factory geladen.");
        String saved="";try{JSONObject setting=vault.get("ntfy:"+api().base);if(setting!=null)saved=setting.optString("endpoint");}catch(Exception ignored){}
        EditText endpoint=input("ntfy-Topic-Adresse (HTTPS)",saved,InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_URI);
        button("Topic in ntfy abonnieren",()->{try{
            String normalized=Api.normalize(endpoint.getText().toString());java.net.URI uri=new java.net.URI(normalized);
            if(!"https".equals(uri.getScheme()) || uri.getPath()==null || uri.getPath().length()<2)throw new Exception("HTTPS-Adresse mit Topic angeben");
            vault.put("ntfy:"+api().base,new JSONObject().put("endpoint",normalized));
            startActivity(new Intent(Intent.ACTION_VIEW,android.net.Uri.parse("ntfy"+normalized.substring(5))));
        }catch(android.content.ActivityNotFoundException e){message("Bitte zuerst die ntfy-App installieren und den Topic dort abonnieren.");}catch(Exception e){message(e.getMessage());}});
        text("Zum Abschalten das Topic in ntfy abbestellen. Zustellung bei Doze und ohne Netz haengt von ntfy, dessen Cache und den Android-Energieeinstellungen ab.");
    }
    private void updates(){
        screen("App-Updates");button("Zurueck",this::projects);message("Pruefe GitHub-Release …");int started=generation;
        background(()->{JSONObject release=Updates.latest();ui(()->{if(generation!=started)return;message("Release "+release.optString("tag"));
            button("Update herunterladen und installieren",()->{
                if(!getPackageManager().canRequestPackageInstalls()){startActivity(new Intent(android.provider.Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,android.net.Uri.parse("package:"+getPackageName())));message("Installation erlauben und danach erneut waehlen");return;}
                message("Download und Signaturpruefung …");background(()->{java.io.File file=Updates.fetch(this,release);ui(()->{if(generation!=started)return;try{
                    Updates.verify(this,file);android.net.Uri uri=android.net.Uri.parse("content://dev.yoloforge.mobile.updates/release-update.apk");startActivity(new Intent(Intent.ACTION_VIEW).setDataAndType(uri,"application/vnd.android.package-archive").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION));
                }catch(Exception e){message(e.getMessage());}});});
            });
        });});
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
        screen("Prompt");images.clear();button("Zurueck",this::project);
        JSONObject draft=null;try{draft=vault.get(draftKey());}catch(Exception e){message("Gespeicherter Entwurf nicht lesbar");}
        prompt=input("Was soll bearbeitet werden?",draft==null?"":draft.optString("prompt"),InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_FLAG_MULTI_LINE);prompt.setMinLines(5);
        if(draft!=null && draft.optJSONArray("images")!=null){JSONArray saved=draft.optJSONArray("images");for(int i=0;i<saved.length();i++)images.add(saved.optJSONObject(i));}
        ArrayList<String> labels=new ArrayList<>(),roots=new ArrayList<>();labels.add("Orchestrator entscheidet");roots.add(null);
        HashSet<String> seen=new HashSet<>();JSONArray commands=array("commands");
        for(int i=commands.length()-1;i>=0;i--){JSONObject c=commands.optJSONObject(i);if(c==null || !c.optString("kind").equals("run_agent"))continue;
            JSONObject spec=c.optJSONObject("agent_spec"),s=find("streams",c.optString("stream_id"));if(spec==null || spec.optBoolean("read_only") || s==null || !s.optString("project_id").equals(projectId))continue;
            JSONObject continuation=spec.optJSONObject("continuation");String root=continuation==null?c.optString("id"):continuation.optString("root_command_id");
            if(seen.add(root)){labels.add(s.optString("goal")+" · "+root.substring(0,8));roots.add(root);}
        }
        String savedRoot=draft==null || draft.isNull("requested_session")?null:draft.optString("requested_session",null);
        if(savedRoot!=null && !roots.contains(savedRoot)){roots.add(savedRoot);labels.add("Gespeicherte feste Session · "+savedRoot);}
        Spinner sessions=new Spinner(this);sessions.setContentDescription("Feste Session oder Orchestrator");sessions.setAdapter(new ArrayAdapter<>(this,android.R.layout.simple_spinner_dropdown_item,labels));sessions.setSelection(Math.max(0,roots.indexOf(savedRoot)));page.addView(sessions);
        composeSessions=sessions;composeRoots=roots;composing=true;
        imageLabel=new TextView(this);imageLabel.setText(getString(R.string.attached_images,images.size()));page.addView(imageLabel);
        if(pickedImage!=null){android.net.Uri uri=pickedImage;pickedImage=null;addImage(uri);}
        button("Bild anhaengen",()->{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE);startActivityForResult(pick,11);});
        button("Bilder entfernen",()->{images.clear();imageLabel.setText("");});
        button("Senden",()->{try{
            String text=prompt.getText().toString();if(text.trim().isEmpty() && images.isEmpty())throw new Exception("Prompt oder Bild eingeben");
            JSONObject body=new JSONObject().put("request_id",UUID.randomUUID().toString()).put("id",JSONObject.NULL).put("expected_revision",0).put("project_id",projectId).put("prompt",text).put("images",new JSONArray(images));
            String root=roots.get(sessions.getSelectedItemPosition());body.put("requested_session",root==null?JSONObject.NULL:root);write("/v1/tasks",body);
        }catch(Exception e){message(e.getMessage());}});
    }
    @Override public void onActivityResult(int request,int result,Intent data){super.onActivityResult(request,result,data);
        if(request==12){if(result==RESULT_OK && data!=null && data.getData()!=null)exportArchiveFile(data.getData());else try{vault.put("archive-export",null);}catch(Exception e){message(e.getMessage());}return;}
        if(request!=11 || result!=RESULT_OK || data==null || data.getData()==null)return;
        if(!composing){pickedImage=data.getData();return;}addImage(data.getData());
    }
    private void addImage(android.net.Uri uri){
        try{
            if(images.size()>=4)throw new Exception("Maximal vier Bilder");String mime=getContentResolver().getType(uri);
            if(!Arrays.asList("image/png","image/jpeg","image/gif","image/webp").contains(mime))throw new Exception("PNG, JPEG, GIF oder WebP verwenden");
            byte[] bytes;try(var in=getContentResolver().openInputStream(uri)){bytes=Api.read(in,4*1024*1024);}
            int total=bytes.length;for(JSONObject image:images)total+=Base64.decode(image.getString("data"),Base64.NO_WRAP).length;
            if(total>4*1024*1024)throw new Exception("Bilder zusammen maximal 4 MiB");
            images.add(new JSONObject().put("name","image-"+(images.size()+1)).put("mime",mime).put("data",Base64.encodeToString(bytes,Base64.NO_WRAP)));
            imageLabel.setText(getString(R.string.attached_images,images.size()));
            saveDraft();
        }catch(Exception e){message(e.getMessage());}
    }
    private void session(JSONObject stream) {
        screen("Session");button("Zurueck",this::project);JSONObject latest=null;JSONArray commands=array("commands");for(int i=commands.length()-1;i>=0;i--){JSONObject c=commands.optJSONObject(i);if(c!=null && c.optString("stream_id").equals(stream.optString("id")) && c.optString("kind").equals("run_agent")){latest=c;break;}}
        if(latest==null){text("Session wird vorbereitet");return;}
        text(latest.optString("state"));JSONObject result=latest.optJSONObject("result");if(result!=null)text(result.toString());
        String id=latest.optString("id");int started=generation;background(()->{JSONObject excerpt=api().request("GET","/v1/commands/"+id+"/excerpt",null);String rendered=excerpt.toString(2);ui(()->{if(generation==started)text(rendered);});});
        button("Vollstaendiges Archiv",()->archive(id,0,null));
    }
    private void archive(String id,long after,Long revision) {
        screen("Archiv");archiveCommand=id;archiveAfter=after;archiveRevision=revision;
        button("Zurueck",this::project);message("Lade Archiv …");int started=generation;
        background(()->{
            JSONObject result=api().request("GET","/v1/commands/"+id+"/archive?after="+after+(revision==null?"":"&through_revision="+revision),null);
            JSONObject state=result.optJSONObject("status");long frozen=revision==null?(state==null?0:state.optLong("revision")):revision;
            JSONArray entries=result.getJSONArray("entries");long next=result.getLong("next_after");
            if(entries.length()>0 && next<=after)throw new Exception("Archivcursor ist nicht fortgeschritten");
            ui(()->{if(generation!=started)return;archiveRevision=frozen;
                message("Archivstand "+frozen+(state!=null && state.optBoolean("final_scan_complete")?" · Scan abgeschlossen":" · noch unvollstaendig"));
                JSONObject termination=result.optJSONObject("termination");if(termination!=null && termination.optBoolean("archive_may_be_incomplete"))text("Die Runtime wurde mit moeglicherweise unvollstaendigem Archiv beendet.");
                for(int i=0;i<entries.length();i++){JSONObject entry=entries.optJSONObject(i);if(entry==null)continue;JSONObject msg=entry.optJSONObject("message");if(msg==null)continue;
                    text("Nachricht "+msg.optString("id")+" · Revision "+entry.optLong("sample_revision"));
                    try{text(ArchiveContent.render(msg));for(JSONObject attachment:ArchiveContent.files(msg)){
                        String label=attachment.optString("name","Anhang");if(!"captured".equals(attachment.optString("status"))){text(label+" · nicht verfuegbar");continue;}
                        button("Anhang speichern: "+label,()->chooseArchiveFile(attachment));
                    }}catch(Exception e){text("Archivinhalt ungueltig: "+e.getMessage());}
                }
                if(entries.length()==0)text("Ende des Archivs");else button("Naechste Seite",()->archive(id,next,frozen));
                if(after>0)button("Zum Anfang",()->archive(id,0,frozen));
                button("Neuen Archivstand laden",()->archive(id,0,null));
            });
        });
    }
    private void chooseArchiveFile(JSONObject file){try{
        ArchiveContent.bytes(file);
        vault.put("archive-export",new JSONObject().put("base",api().base).put("file",file));
        startActivityForResult(new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/octet-stream").putExtra(Intent.EXTRA_TITLE,ArchiveContent.safeName(file.optString("name"))),12);
    }catch(Exception e){message(e.getMessage());}}
    private void exportArchiveFile(android.net.Uri uri){background(()->{
        JSONObject pending=vault.get("archive-export");if(pending==null)throw new Exception("Kein Anhang zum Export gespeichert");
        if(!api().base.equals(pending.getString("base")))throw new Exception("Anhang gehoert zu einer anderen Factory");
        byte[] bytes=ArchiveContent.bytes(pending.getJSONObject("file"));
        try(var out=getContentResolver().openOutputStream(uri,"wt")){if(out==null)throw new Exception("Ausgabedatei nicht verfuegbar");out.write(bytes);out.flush();}
        vault.put("archive-export",null);ui(()->message("Anhang gespeichert und SHA256 geprueft"));
    });}
    private void questions() {
        screen("Rueckfragen");button("Projekte",this::projects);button("Aktualisieren",()->{deepQuestion=null;projectId=null;refresh();});
        JSONArray questions=array("questions");int count=0;for(int i=0;i<questions.length();i++){JSONObject q=questions.optJSONObject(i);if(q==null || !Arrays.asList("pending","rejected").contains(q.optString("state")))continue;count++;
            JSONObject ext=q.optJSONObject("external"),payload=ext==null?null:ext.optJSONObject("payload");button(payload==null?"Rueckfrage":payload.optString("title",payload.optString("action","Rueckfrage")),()->answer(q));}
        if(count==0)text("Keine offenen Rueckfragen");
    }
    private void answer(JSONObject q) {
        screen("Antwort");button("Zurueck",this::questions);text(q.optString("orchestration_note"));
        if(!Arrays.asList("pending","rejected").contains(q.optString("state"))){text("Diese Frage wurde bereits beantwortet oder geschlossen.");return;}
        JSONObject ext=q.optJSONObject("external"),payload=ext==null?null:ext.optJSONObject("payload");if(payload==null)return;
        text(payload.optString("title",payload.optString("action")));text(payload.optString("description"));
        if("permission".equals(ext.optString("kind"))){text(payload.optJSONArray("resources")==null?"":payload.optJSONArray("resources").toString());
            button("Einmal erlauben",()->permission(q,"once"));button("Ablehnen",()->permission(q,"reject"));return;}
        JSONArray fields=payload.optJSONArray("fields");if(fields==null)return;
        try{form=new FormEditor(this,page,fields,vault.get(answerDraftKey(q)));editingQuestion=q;}
        catch(Exception e){text(e.getMessage());return;}
        button("Antwort senden",()->{try{sendAnswer(q,form.answer());}catch(Exception e){message(e.getMessage());}});
    }
    private void permission(JSONObject q,String decision){try{sendAnswer(q,new JSONObject().put("decision",decision));}catch(Exception e){message(e.getMessage());}}
    private void sendAnswer(JSONObject q,JSONObject answer)throws Exception{write("/v1/questions/"+q.getString("id")+"/answer",new JSONObject().put("request_id",UUID.randomUUID().toString()).put("expected_revision",q.getLong("revision")).put("answer",answer));}
    private void write(String path,JSONObject body)throws Exception {
        if(vault.get("pending")!=null){pendingScreen();return;}
        new Outbox(vault).prepare(api(),path,body);
        if(composing){vault.put(draftKey(),null);composing=false;}
        pendingScreen();retry();
    }
    private void pendingScreen(){screen("Auftrag gespeichert");text("Bei Verbindungsabbruch bleibt der identische Auftrag mit derselben Request-ID gespeichert. Erneutes Senden erzeugt keinen zweiten Auftrag.");button("Erneut senden",this::retry);button("Neu anmelden",this::login);}
    private void retry(){message("Sende gespeicherten Auftrag …");background(()->{
        JSONObject pending=vault.get("pending");if(pending==null){ui(this::refresh);return;}Api client=api();if(!client.base.equals(pending.getString("base")))throw new Exception("Auftrag gehoert zu einer anderen Factory");
        try{new Outbox(vault).send(client);ui(this::refresh);}
        catch(Api.HttpFailure e){if(Arrays.asList(400,403,404,409,422).contains(e.status)){ui(()->{projects();message("Auftrag abgelehnt (HTTP "+e.status+"). Bitte aktualisieren und korrigieren.");});}else throw e;}
    });}
}
