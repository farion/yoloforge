package dev.yoloforge.mobile;

import android.Manifest;
import android.app.AlertDialog;
import android.content.Intent;
import android.os.Bundle;
import android.text.InputType;
import android.util.Base64;
import android.view.View;
import android.view.Gravity;
import android.widget.*;
import java.util.*;
import java.util.concurrent.Executors;
import org.json.*;
import static dev.yoloforge.mobile.FactoryStyle.*;

public final class MainActivity extends androidx.activity.ComponentActivity {
    private final java.util.concurrent.ExecutorService worker = Executors.newSingleThreadExecutor();
    private Vault vault;
    private LinearLayout page;
    private FrameLayout loading;
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
    private String deepProblem;
    private FrameLayout frame;
    private LinearLayout surface;
    private FrameLayout flyout;
    private boolean flyoutClosing;
    private FrameLayout projectOverlay;
    private Runnable restoreProjectChooser;
    private final Map<String,List<String>> projectDrafts=new HashMap<>();
    private Runnable saveProjectDraft;
    private String screenTitle;
    private enum Section { OVERVIEW, INTENTS, INBOX, SESSIONS, ACTIVITY, NOTIFICATIONS, INFRASTRUCTURE, DIAGNOSE }
    private Section section=Section.OVERVIEW;
    private boolean sessionArchive;
    private int activityRole;
    private long unreadNotifications;
    private boolean overviewOnlyProject;
    private boolean online;
    private TextView connectionLabel, factoryConnectionBadge;
    private ImageView connectionSymbol;
    private final android.os.Handler connectionHandler=new android.os.Handler(android.os.Looper.getMainLooper());
    private int connectionWatch;
    private final Runnable connectionProbe=this::probeConnection;
    private int inboxTab, inboxFilter;
    private String inboxQuery="";
    private final androidx.activity.OnBackPressedCallback drawerBack = new androidx.activity.OnBackPressedCallback(false) {
        @Override public void handleOnBackPressed() {
            if(projectOverlay!=null)closeProjectDropdown();else dismissFlyout();
        }
    };
    private final androidx.activity.OnBackPressedCallback projectEditorBack = new androidx.activity.OnBackPressedCallback(false) {
        @Override public void handleOnBackPressed() { returnToProjectMenu(); }
    };
    private void readIntent(Intent intent){
        deepQuestion=intent.getStringExtra("question");deepProblem=null;showQuestions=false;
        android.net.Uri uri=intent.getData();
        if(uri==null || !"yoloforge".equals(uri.getScheme()))return;
        showQuestions="questions".equals(uri.getHost());
        if("inbox".equals(uri.getHost())){
            List<String> parts=uri.getPathSegments();
            if(parts.size()!=3 || !InboxPush.accepts(this,parts.get(0)))return;
            if("question".equals(parts.get(1)))deepQuestion=parts.get(2);
            if("problem".equals(parts.get(1)))deepProblem=parts.get(2);
        }
    }
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
            state.putString("section",section.name());state.putBoolean("sessionArchive",sessionArchive);
            state.putInt("activityRole",activityRole);state.putBoolean("overviewOnlyProject",overviewOnlyProject);
            state.putInt("inboxTab",inboxTab);state.putInt("inboxFilter",inboxFilter);state.putString("inboxQuery",inboxQuery);
            state.putBoolean("inbox","Inbox".equals(screenTitle));
            state.putBoolean("compose",composing || restoreCompose);
            state.putString("archive",archiveCommand);
            state.putLong("archiveAfter",archiveAfter);
            if(archiveRevision!=null)state.putLong("archiveRevision",archiveRevision);
        }catch(Exception e){message("Ansicht konnte nicht gespeichert werden");}
        super.onSaveInstanceState(state);
    }

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved); vault = new Vault(this);readIntent(getIntent());
        getOnBackPressedDispatcher().addCallback(this,projectEditorBack);
        getOnBackPressedDispatcher().addCallback(this,drawerBack);
        if(saved!=null){restoreBase=saved.getString("factory");projectId=saved.getString("project");restoreCompose=saved.getBoolean("compose");restoreQuestion=saved.getString("answer");archiveCommand=saved.getString("archive");archiveAfter=saved.getLong("archiveAfter");if(saved.containsKey("archiveRevision"))archiveRevision=saved.getLong("archiveRevision");inboxTab=saved.getInt("inboxTab");inboxFilter=saved.getInt("inboxFilter");inboxQuery=saved.getString("inboxQuery","");showQuestions|=saved.getBoolean("inbox");}
        if(saved!=null){section=Section.valueOf(saved.getString("section",Section.OVERVIEW.name()));sessionArchive=saved.getBoolean("sessionArchive");activityRole=saved.getInt("activityRole");overviewOnlyProject=saved.getBoolean("overviewOnlyProject");}
        try { if (vault.get("credentials") == null) login(); else refresh(); } catch (Exception error) { login(); message(error.getMessage()); }
    }
    @Override public void onNewIntent(Intent intent) { super.onNewIntent(intent);setIntent(intent);readIntent(intent);refresh(); }
    @Override protected void onStart(){super.onStart();connectionHandler.post(connectionProbe);}
    @Override protected void onStop(){connectionWatch++;connectionHandler.removeCallbacks(connectionProbe);super.onStop();}
    @Override public void onDestroy() { cancelOverviewExpiry(); worker.shutdown(); super.onDestroy(); }
    private void probeConnection(){
        int watch=connectionWatch, started=generation;
        if(snapshot==null||"Anmeldung".equals(screenTitle)){
            connectionHandler.postDelayed(connectionProbe,10000);return;
        }
        worker.execute(()->{
            boolean reachable;
            try{reachable=api().healthy();}
            catch(Exception e){reachable=false;}
            boolean result=reachable;
            ui(()->{
                if(watch!=connectionWatch)return;
                if(started==generation&&result!=online){setOnline(result);toast(result?"Wieder mit Factory verbunden.":"Factory nicht erreichbar.");}
                connectionHandler.postDelayed(connectionProbe,10000);
            });
        });
    }
    private void setOnline(boolean connected){
        online=connected;
        if(connectionLabel!=null){
            String current=connectionLabel.getText().toString();
            int separator=current.indexOf(" · ");
            connectionLabel.setText((online?"Verbunden":"Offline")+(separator<0?"":current.substring(separator)));
            connectionLabel.setTextColor(online?GREEN:MUTED);
            connectionSymbol.setColorFilter(online?GREEN:MUTED);
        }
        if(factoryConnectionBadge!=null){
            TextView style=pill(this,online?"Online":"Offline",online?GREEN:MUTED);
            factoryConnectionBadge.setText(style.getText());
            factoryConnectionBadge.setTextColor(style.getCurrentTextColor());
            factoryConnectionBadge.setBackground(style.getBackground());
        }
        if(flyout!=null){
            View pause=flyout.findViewWithTag("project-pause");
            if(pause!=null){pause.setEnabled(online);pause.setAlpha(online?1f:.5f);}
        }
    }
    private void screen(String title) {
        cancelOverviewExpiry();
        if(saveProjectDraft!=null){saveProjectDraft.run();saveProjectDraft=null;}
        projectEditorBack.setEnabled(false);
        try{saveDraft();saveAnswerDraft();}catch(Exception e){android.widget.Toast.makeText(this,"Entwurf konnte nicht gespeichert werden",android.widget.Toast.LENGTH_LONG).show();}
        form=null;editingQuestion=null;
        composing=false;
        if(!title.equals("Archiv") && !title.equals("Verbinden"))archiveCommand=null;
        generation++;
        screenTitle=title;closeFlyout();
        frame=new FrameLayout(this);frame.setBackgroundColor(PANEL);
        frame.setOnApplyWindowInsetsListener((v,insets)->{
            if(android.os.Build.VERSION.SDK_INT>=30){
                var edges=insets.getInsets(android.view.WindowInsets.Type.systemBars()|android.view.WindowInsets.Type.displayCutout()|android.view.WindowInsets.Type.ime());
                v.setPadding(edges.left,edges.top,edges.right,edges.bottom);
            }else v.setPadding(insets.getSystemWindowInsetLeft(),insets.getSystemWindowInsetTop(),insets.getSystemWindowInsetRight(),insets.getSystemWindowInsetBottom());
            return insets;
        });
        surface=new LinearLayout(this);surface.setOrientation(LinearLayout.VERTICAL);
        int available=getResources().getConfiguration().screenWidthDp;
        frame.addView(surface,new FrameLayout.LayoutParams(available>960?dp(960):-1,-1,android.view.Gravity.CENTER_HORIZONTAL));
        setContentView(frame);
        LinearLayout header=new LinearLayout(this);header.setGravity(Gravity.CENTER_VERTICAL);
        header.setBackgroundColor(SHELL); header.setPadding(dp(4),0,dp(12),0);
        surface.addView(header,new LinearLayout.LayoutParams(-1,dp(54)));
        if(snapshot!=null && !title.equals("Anmeldung")){
            iconButton(header,R.drawable.ic_menu,"Navigation öffnen",this::openFlyout);
        } else {
            ImageView logo=icon(R.drawable.ic_factory,TEXT);
            LinearLayout.LayoutParams logoParams=new LinearLayout.LayoutParams(dp(22),dp(22));
            logoParams.setMargins(dp(12),0,dp(10),0);header.addView(logo,logoParams);
        }
        JSONObject currentProject=find("projects",projectId);
        boolean projectView=Arrays.asList("Intents","Inbox","Sessions","Projektaktivität").contains(title);
        String breadcrumb=projectView&&currentProject!=null?currentProject.optString("name"):"Factory";
        TextView heading=label(this,"Anmeldung".equals(title)?getString(R.string.page_title,title):breadcrumb+"  ›  "+title,19,TEXT);
        truncate(heading);
        header.addView(heading,new LinearLayout.LayoutParams(0,-2,1));
        if("Benachrichtigungen".equals(title)){
            ImageButton menu=iconButton(header,R.drawable.ic_dots_three,"Weitere Aktionen",null);
            menu.setOnClickListener(v->{
                PopupMenu actions=new PopupMenu(this,menu);
                actions.getMenu().add("Aktualisieren").setOnMenuItemClickListener(item->{refresh();return true;});
                actions.getMenu().add("Alles als gelesen markieren").setEnabled(unreadNotifications>0)
                    .setOnMenuItemClickListener(item->{markAllNotificationsRead();return true;});
                actions.show();
            });
        }else if("Inbox".equals(title)) iconButton(header,R.drawable.ic_arrow_clockwise,"Aktualisieren",()->{showQuestions=true;refresh();});
        else if(Arrays.asList("Overview","Intents","Sessions","Projektaktivität","Infrastructure","Diagnose").contains(title))
            iconButton(header,R.drawable.ic_arrow_clockwise,"Aktualisieren",this::refresh);
        divider(surface);
        loading=null;connectionLabel=null;connectionSymbol=null;factoryConnectionBadge=null;
        ScrollView scroll = new ScrollView(this);scroll.setFillViewport(true);surface.addView(scroll,new LinearLayout.LayoutParams(-1,0,1));
        page = new LinearLayout(this); page.setOrientation(LinearLayout.VERTICAL);page.setPadding(dp(14),dp(14),dp(14),dp(14));scroll.addView(page);
        if(snapshot!=null && !"Anmeldung".equals(title)) connectionFooter(surface);
    }
    private int dp(int n){return FactoryStyle.dp(this,n);}
    private void divider(LinearLayout container){
        View line=new View(this);line.setBackgroundColor(LINE);container.addView(line,new LinearLayout.LayoutParams(-1,dp(1)));
    }
    private static void truncate(TextView label){label.setSingleLine(true);label.setEllipsize(android.text.TextUtils.TruncateAt.END);}
    private ImageView icon(int resource,int color){ImageView icon=new ImageView(this);icon.setImageResource(resource);icon.setColorFilter(color);return icon;}
    private ImageButton iconButton(LinearLayout target,int resource,String label,Runnable action){
        ImageButton button=new ImageButton(this);button.setImageResource(resource);button.setColorFilter(TEXT);
        button.setContentDescription(label);button.setTooltipText(label);button.setBackground(interactive(this,0x00000000,4));
        target.addView(button,new LinearLayout.LayoutParams(dp(46),dp(46)));
        if(action!=null)button.setOnClickListener(v->action.run());
        return button;
    }
    private void connectionFooter(LinearLayout target){
        divider(target);
        LinearLayout footer=new LinearLayout(this);footer.setGravity(Gravity.CENTER_VERTICAL);footer.setBackgroundColor(SHELL);
        footer.setPadding(dp(14),0,dp(4),0);target.addView(footer,new LinearLayout.LayoutParams(-1,dp(46)));
        int color=online?GREEN:MUTED;
        ImageView symbol=icon(R.drawable.ic_plugs_connected,color);
        connectionSymbol=symbol;
        footer.addView(symbol,new LinearLayout.LayoutParams(dp(16),dp(16)));
        String source="";try{source=android.net.Uri.parse(api().base).getAuthority();}catch(Exception ignored){}
        TextView connection=label(this,(online?"Verbunden":"Offline")+(source==null||source.isEmpty()?"":" · "+source),12,color);
        connectionLabel=connection;
        truncate(connection);connection.setPaddingRelative(dp(8),0,0,0);
        footer.addView(connection,new LinearLayout.LayoutParams(0,-2,1));
    }
    private void openFlyout(){
        if(flyout!=null)return;
        drawerBack.setEnabled(true);
        flyout=new FrameLayout(this);flyout.setBackgroundColor(0x99000000);flyout.setOnClickListener(v->dismissFlyout());
        frame.addView(flyout,new FrameLayout.LayoutParams(-1,-1));
        surface.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
        ScrollView scroll=new ScrollView(this);scroll.setBackgroundColor(SHELL);scroll.setFillViewport(true);scroll.setClickable(true);
        int width=frame.getWidth()-frame.getPaddingLeft()-frame.getPaddingRight();
        if(width<=0)width=dp(getResources().getConfiguration().screenWidthDp);
        int drawerWidth=Math.min(dp(304),Math.max(dp(160),width-dp(52)));
        flyout.addView(scroll,new FrameLayout.LayoutParams(drawerWidth,-1,Gravity.START));
        boolean rtl=frame.getLayoutDirection()==View.LAYOUT_DIRECTION_RTL;
        scroll.setTranslationX(rtl?drawerWidth:-drawerWidth);
        flyout.setAlpha(0);
        scroll.animate().translationX(0).setDuration(220).start();
        flyout.animate().alpha(1).setDuration(220).start();
        LinearLayout rows=new LinearLayout(this);rows.setOrientation(LinearLayout.VERTICAL);rows.setPadding(dp(16),dp(14),dp(16),dp(14));scroll.addView(rows);
        sectionLabel(rows,"PROJEKT");
        JSONObject project=find("projects",projectId);
        Button chooser=FactoryStyle.button(this,project==null?"Projekt auswählen":projectLabel(project),false);
        chooser.setTooltipText("Projekt auswählen");
        chooser.setCompoundDrawablesRelativeWithIntrinsicBounds(0,0,R.drawable.ic_caret_down,0);
        chooser.setGravity(Gravity.START|Gravity.CENTER_VERTICAL);truncate(chooser);
        LinearLayout controls=new LinearLayout(this);controls.setGravity(Gravity.CENTER_VERTICAL);
        controls.addView(chooser,new LinearLayout.LayoutParams(0,-2,1));
        if(project!=null){
            boolean paused=project.optBoolean("paused");
            ImageButton pause=iconButton(controls,paused?R.drawable.ic_play:R.drawable.ic_pause,
                paused?"Projekt fortsetzen":"Projekt pausieren",()->toggleProjectPause(project));
            pause.setTag("project-pause");
            pause.setEnabled(online);
            if(!online)pause.setAlpha(.5f);
        }
        addControl(rows,controls,true);
        chooser.setEnabled(array("projects").length()>0);
        chooser.setOnClickListener(v->openProjectDropdown(scroll,chooser));
        intentShortcut(rows,project);
        navigationRow(rows,R.drawable.ic_list_checks,"Intents",section==Section.INTENTS,0,this::project);
        navigationRow(rows,R.drawable.ic_tray,"Inbox",section==Section.INBOX,inboxCount(false,0),this::questions);
        navigationRow(rows,R.drawable.ic_terminal_window,"Sessions",section==Section.SESSIONS,0,()->sessionList(false));
        navigationRow(rows,R.drawable.ic_pulse,"Projektaktivität",section==Section.ACTIVITY,0,()->sessionList(true));
        View space=new View(this);rows.addView(space,new LinearLayout.LayoutParams(-1,dp(12)));divider(rows);
        sectionLabel(rows,"FACTORY");
        navigationRow(rows,R.drawable.ic_squares_four,"Overview",section==Section.OVERVIEW,0,this::overview);
        navigationRow(rows,R.drawable.ic_bell,"Benachrichtigungen",section==Section.NOTIFICATIONS,(int)Math.min(Integer.MAX_VALUE,unreadNotifications),this::factoryNotifications);
        navigationRow(rows,R.drawable.ic_hard_drives,"Infrastructure",section==Section.INFRASTRUCTURE,0,this::infrastructure);
        navigationRow(rows,R.drawable.ic_chart_line,"Diagnose",section==Section.DIAGNOSE,0,this::diagnostics);
        View appSpace=new View(this);rows.addView(appSpace,new LinearLayout.LayoutParams(-1,dp(12)));divider(rows);
        sectionLabel(rows,"APP");
        navigationRow(rows,R.drawable.ic_gear,"App-Einstellungen",false,0,this::appSettings);
    }
    private String projectLabel(JSONObject project){
        String key=project.optString("key");
        return key.isEmpty()?project.optString("name"):key+" · "+project.optString("name");
    }
    private void openProjectDropdown(ScrollView navigation,Button chooser){
        if(projectOverlay!=null || flyoutClosing)return;
        JSONArray projects=array("projects");
        LinearLayout options=projectOptions();
        TextView focusOption=null;
        for(int i=0;i<projects.length();i++){
            JSONObject p=projects.optJSONObject(i);if(p==null)continue;
            String id=p.optString("id");
            boolean selected=id.equals(projectId);
            TextView option=row(options,projectLabel(p),selected,()->{
                closeProjectDropdown();
                if(id.equals(projectId))return;
                projectId=id;
                try{vault.put("project-choice:"+api().base,new JSONObject().put("id",id));}
                catch(Exception ignored){toast("Projektauswahl konnte nicht gespeichert werden.");}
                if(Arrays.asList(Section.INTENTS,Section.INBOX,Section.SESSIONS,Section.ACTIVITY).contains(section)
                    || section==Section.OVERVIEW&&overviewOnlyProject){
                    Section current=section;
                    closeFlyout();showSection();
                    frame.post(()->{if(section==current)openFlyout();});
                }else{
                    // Global pages stay visible. Rebuild only the flyout to update its logo and badges.
                    closeFlyout();openFlyout();
                }
            });
            option.setContentDescription(p.optString("name"));
            option.setSelected(selected);
            if(focusOption==null || selected)focusOption=option;
        }
        if(focusOption!=null)showProjectOverlay(navigation,chooser,chooser,options,focusOption);
    }
    private LinearLayout projectOptions(){
        LinearLayout options=new LinearLayout(this);options.setOrientation(LinearLayout.VERTICAL);
        options.setPadding(dp(4),dp(4),dp(4),0);
        return options;
    }
    private void showProjectOverlay(ScrollView navigation,View anchorView,View trigger,LinearLayout options,View focusOption){
        if(projectOverlay!=null || flyoutClosing)return;
        navigation.animate().cancel();navigation.setTranslationX(0);
        int[] anchor=new int[2], drawer=new int[2];
        anchorView.getLocationInWindow(anchor);flyout.getLocationInWindow(drawer);
        int top=anchor[1]-drawer[1], width=anchorView.getWidth();
        int gap=dp(2), below=flyout.getHeight()-top-anchorView.getHeight()-gap, above=top-gap;
        int available=Math.max(above,below);
        if(width<=0 || available<=0)return;
        ScrollView dropdown=new ScrollView(this);
        dropdown.setBackground(fill(this,INPUT,4));dropdown.setElevation(dp(8));dropdown.setClickable(true);
        dropdown.addView(options);
        dropdown.measure(View.MeasureSpec.makeMeasureSpec(width,View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(Math.min(dp(216),available),View.MeasureSpec.AT_MOST));
        int height=dropdown.getMeasuredHeight();
        FrameLayout.LayoutParams region=new FrameLayout.LayoutParams(width,height,Gravity.TOP|Gravity.LEFT);
        region.leftMargin=anchor[0]-drawer[0];
        region.topMargin=height<=below?top+anchorView.getHeight()+gap:top-gap-height;
        projectOverlay=new FrameLayout(this);
        projectOverlay.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        projectOverlay.setOnClickListener(v->closeProjectDropdown());
        projectOverlay.addView(dropdown,region);
        flyout.addView(projectOverlay,new FrameLayout.LayoutParams(-1,-1));
        trigger.setSelected(true);
        navigation.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
        restoreProjectChooser=()->{
            navigation.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_AUTO);
            trigger.setSelected(false);
            if(!trigger.isInTouchMode())trigger.requestFocus();
        };
        if(!trigger.isInTouchMode())focusOption.requestFocus();
    }
    private void closeProjectDropdown(){
        if(projectOverlay==null)return;
        flyout.removeView(projectOverlay);projectOverlay=null;
        restoreProjectChooser.run();restoreProjectChooser=null;
    }
    private static String projectPath(JSONObject project){return "/v1/projects/"+project.optString("id");}
    private void returnToProjectMenu(){
        showSection();frame.post(this::openFlyout);
    }
    private EditText projectField(String caption,String hint,String value){
        TextView title=label(this,caption,14,TEXT);page.addView(title);
        EditText edit=input(hint,value,InputType.TYPE_CLASS_TEXT);edit.setSingleLine(true);
        edit.setId(View.generateViewId());title.setLabelFor(edit.getId());
        ((LinearLayout.LayoutParams)edit.getLayoutParams()).bottomMargin=dp(14);
        return edit;
    }
    private void projectEditor(JSONObject project){
        boolean create=project==null;
        String path=create?"/v1/projects":projectPath(project), draftKey;
        try{draftKey=api().base+path;}catch(Exception e){toast("Bitte zuerst anmelden.");return;}
        screen(create?"Neues Projekt":"Projekteinstellungen");
        projectEditorBack.setEnabled(true);
        iconButton((LinearLayout)surface.getChildAt(0),R.drawable.ic_x,"Schließen",this::returnToProjectMenu);
        List<String> draft=projectDrafts.getOrDefault(draftKey,create?Arrays.asList("","","factory:managed","main"):
            Arrays.asList(project.optString("name"),project.optString("key"),project.optString("repository"),project.optString("base_branch")));
        EditText name=projectField("Projektname","Name des Projekts",draft.get(0));
        EditText key=projectField(create?"Kürzel (optional)":"Kürzel","z. B. PROJ",draft.get(1));
        EditText repository=projectField("Repository","factory:managed",draft.get(2));
        EditText branch=projectField("Basisbranch","main",draft.get(3));
        saveProjectDraft=()->projectDrafts.put(draftKey,Arrays.asList(name.getText().toString(),key.getText().toString(),
            repository.getText().toString(),branch.getText().toString()));
        LinearLayout footer=new LinearLayout(this);footer.setGravity(Gravity.END);
        footer.setPadding(dp(14),dp(6),dp(14),dp(6));footer.setBackgroundColor(SHELL);
        int footerIndex=surface.indexOfChild((View)page.getParent())+1;
        surface.addView(footer,footerIndex);
        View line=new View(this);line.setBackgroundColor(LINE);
        surface.addView(line,footerIndex,new LinearLayout.LayoutParams(-1,dp(1)));
        Button save=FactoryStyle.button(this,"Speichern",true);footer.addView(save);
        save.setOnClickListener(v->{
            String title=name.getText().toString().trim(), prefix=key.getText().toString().trim().toUpperCase(Locale.ROOT);
            String repo=repository.getText().toString().trim(), base=branch.getText().toString().trim();
            if(title.isEmpty()){name.setError("Projektname fehlt.");return;}
            if(!prefix.isEmpty() && !prefix.matches("[A-Z][A-Z0-9]{0,9}")){key.setError("1–10 Buchstaben oder Ziffern, beginnend mit einem Buchstaben.");return;}
            if(repo.isEmpty()){repository.setError("Repository fehlt.");return;}
            if(base.isEmpty()){branch.setError("Basisbranch fehlt.");return;}
            try{
                JSONObject body=new JSONObject().put("request_id",UUID.randomUUID().toString()).put("name",title)
                    .put("repository",repo).put("base_branch",base);
                if(!prefix.isEmpty())body.put("key",prefix);
                if(!create)body.put("expected_revision",project.getLong("revision"));
                write(create?"POST":"PATCH",path,body);
            }catch(Exception e){toast("Projekt konnte nicht gespeichert werden.");}
        });
    }
    private void confirmProjectDeletion(JSONObject project){
        closeProjectDropdown();
        String name=project.optString("name");
        LinearLayout content=new LinearLayout(this);content.setOrientation(LinearLayout.VERTICAL);
        content.setPadding(dp(24),dp(8),dp(24),dp(8));
        content.addView(label(this,"„"+name+"“ aus der Factory entfernen? Repository-Dateien und Historie bleiben erhalten. Laufende oder eingereihte Aufträge verhindern das Löschen.",14,TEXT));
        EditText confirmation=new EditText(this);FactoryStyle.input(confirmation,"Vollständiger Projektname");
        confirmation.setContentDescription("Vollständiger Projektname");confirmation.setSingleLine(true);
        LinearLayout.LayoutParams field=new LinearLayout.LayoutParams(-1,-2);field.topMargin=dp(12);content.addView(confirmation,field);
        ScrollView scroll=new ScrollView(this);scroll.addView(content);
        AlertDialog dialog=new AlertDialog.Builder(this).setTitle("Projekt löschen").setView(scroll)
            .setNegativeButton("Abbrechen",null).setPositiveButton("Löschen",null).create();
        dialog.show();
        Button delete=dialog.getButton(AlertDialog.BUTTON_POSITIVE);delete.setEnabled(false);
        confirmation.addTextChangedListener(new android.text.TextWatcher(){
            @Override public void beforeTextChanged(CharSequence s,int start,int count,int after){}
            @Override public void onTextChanged(CharSequence s,int start,int before,int count){delete.setEnabled(name.contentEquals(s));}
            @Override public void afterTextChanged(android.text.Editable s){}
        });
        delete.setOnClickListener(v->{
            try{
                write("DELETE",projectPath(project),new JSONObject().put("request_id",UUID.randomUUID().toString())
                    .put("expected_revision",project.getLong("revision")).put("confirm_name",confirmation.getText().toString()));
                dialog.dismiss();
            }catch(Exception e){toast("Projekt konnte nicht gelöscht werden.");}
        });
    }
    private void toggleProjectPause(JSONObject project){
        try{
            write(projectPath(project)+"/pause",new JSONObject().put("request_id",UUID.randomUUID().toString())
                .put("expected_revision",project.getLong("revision")).put("paused",!project.optBoolean("paused")));
        }catch(Exception e){toast("Projektstatus konnte nicht geändert werden.");}
    }
    private void intentShortcut(LinearLayout parent,JSONObject project){
        LinearLayout row=new LinearLayout(this);row.setGravity(Gravity.CENTER_VERTICAL);
        addControl(parent,row,true);
        android.graphics.Bitmap bitmap=projectLogo(project);
        if(bitmap!=null){
            ImageView logo=new ImageView(this);logo.setImageBitmap(bitmap);logo.setScaleType(ImageView.ScaleType.CENTER_CROP);
            logo.setBackground(fill(this,CELL,4));logo.setClipToOutline(true);logo.setContentDescription("Projektlogo");
            LinearLayout.LayoutParams logoParams=new LinearLayout.LayoutParams(dp(55),dp(55));
            logoParams.setMarginEnd(dp(8));row.addView(logo,logoParams);
        }
        Button create=FactoryStyle.button(this,"Intent",true);
        android.graphics.drawable.Drawable plus=getDrawable(R.drawable.ic_plus);
        plus.setBounds(0,0,dp(19),dp(19));
        android.text.SpannableString caption=new android.text.SpannableString("\ufffc  Intent");
        caption.setSpan(new android.text.style.ImageSpan(plus,android.text.style.ImageSpan.ALIGN_BOTTOM),
            0,1,android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        create.setText(caption);
        create.setPaddingRelative(dp(14),0,dp(14),0);
        create.setTextSize(17);create.setContentDescription("Neuer Intent");
        row.addView(create,new LinearLayout.LayoutParams(0,dp(55),1));
        create.setEnabled(project!=null);if(project==null)create.setAlpha(.5f);
        create.setOnClickListener(v->{closeFlyout();section=Section.INTENTS;compose();});
    }
    static android.graphics.Bitmap projectLogo(JSONObject project){
        JSONObject logo=project==null?null:project.optJSONObject("logo");
        if(logo==null)return null;
        String encoded=logo.optString("png_base64");
        if(encoded.isEmpty()||encoded.length()>131072)return null;
        try{
            byte[] data=Base64.decode(encoded,Base64.DEFAULT);
            android.graphics.BitmapFactory.Options size=new android.graphics.BitmapFactory.Options();
            size.inJustDecodeBounds=true;android.graphics.BitmapFactory.decodeByteArray(data,0,data.length,size);
            if(size.outWidth!=128||size.outHeight!=128||!"image/png".equals(size.outMimeType))return null;
            return android.graphics.BitmapFactory.decodeByteArray(data,0,data.length);
        }catch(IllegalArgumentException ignored){return null;}
    }
    private void dismissFlyout(){
        FrameLayout drawer=flyout;
        if(drawer==null || flyoutClosing)return;
        flyoutClosing=true;
        View panel=drawer.getChildAt(0);
        int direction=frame.getLayoutDirection()==View.LAYOUT_DIRECTION_RTL?1:-1;
        panel.animate().translationX(direction*panel.getWidth()).setDuration(180).start();
        drawer.animate().alpha(0).setDuration(180).withEndAction(()->{
            if(flyout==drawer)closeFlyout();
        }).start();
    }
    private void closeFlyout(){
        if(flyout==null)return;
        closeProjectDropdown();
        flyout.animate().cancel();
        flyout.getChildAt(0).animate().cancel();
        frame.removeView(flyout);flyout=null;flyoutClosing=false;drawerBack.setEnabled(false);surface.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_AUTO);
    }
    private void sectionLabel(LinearLayout rows,String title){
        TextView label=label(this,title,12,MUTED);label.setPaddingRelative(0,dp(9),0,dp(6));rows.addView(label);
    }
    private void navigationRow(LinearLayout rows,int resource,String title,boolean selected,int count,Runnable action){
        LinearLayout view=new LinearLayout(this);view.setGravity(Gravity.CENTER_VERTICAL);
        view.setBackground(interactive(this,selected?SELECTED:0x00000000,4));
        view.setMinimumHeight(dp(44));view.setPaddingRelative(0,0,dp(10),0);
        View marker=new View(this);marker.setBackground(fill(this,selected?BLUE:0,2));
        view.addView(marker,new LinearLayout.LayoutParams(dp(3),dp(24)));
        ImageView symbol=icon(resource,selected?TEXT:MUTED);
        LinearLayout.LayoutParams iconParams=new LinearLayout.LayoutParams(dp(19),dp(19));iconParams.setMargins(dp(10),0,dp(10),0);view.addView(symbol,iconParams);
        TextView name=label(this,title,15,selected?TEXT:MUTED);truncate(name);
        view.addView(name,new LinearLayout.LayoutParams(0,-2,1));
        if(count>0)view.addView(pill(this,Integer.toString(count),AMBER));
        view.setContentDescription(title);view.setFocusable(true);view.setSelected(selected);
        view.setOnClickListener(v->{closeFlyout();action.run();});
        addControl(rows,view,true);
    }
    private TextView row(LinearLayout rows,String label,boolean selected,Runnable action){
        TextView view=new TextView(this);view.setText(label);view.setTextSize(16);view.setMinHeight(dp(48));
        view.setGravity(android.view.Gravity.CENTER_VERTICAL);view.setPadding(dp(12),dp(8),dp(12),dp(8));
        view.setMaxLines(2);view.setEllipsize(android.text.TextUtils.TruncateAt.END);
        android.util.TypedValue value=new android.util.TypedValue();getTheme().resolveAttribute(android.R.attr.selectableItemBackground,value,true);
        view.setForeground(getDrawable(value.resourceId));view.setFocusable(true);
        view.setTextColor(TEXT);view.setBackground(fill(this,selected?SELECTED:CELL,2));
        LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(-1,-2);params.bottomMargin=dp(4);rows.addView(view,params);
        view.setOnClickListener(v->action.run());
        return view;
    }
    private void text(String value) { if(value==null||value.isEmpty())return;TextView view=label(this,value,15,TEXT);view.setTextIsSelectable(true);view.setPadding(0,dp(5),0,dp(5));page.addView(view); }
    private void addControl(LinearLayout target,View view,boolean full){
        LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(full?-1:-2,-2);params.bottomMargin=dp(6);target.addView(view,params);
    }
    private void button(String label,Runnable action){
        boolean primary=Arrays.asList("Neuer Prompt","Senden","Antwort senden","Einmal erlauben").contains(label);
        Button button=FactoryStyle.button(this,label,primary);button.setOnClickListener(v->action.run());addControl(page,button,false);
    }
    private EditText input(String hint,String value,int type){
        EditText edit=new EditText(this);edit.setContentDescription(hint);edit.setInputType(type);edit.setText(value);
        FactoryStyle.input(edit,hint);addControl(page,edit,true);return edit;
    }
    private void loading(boolean active){
        if(loading!=null){frame.removeView(loading);loading=null;}
        if(!active)return;
        loading=new FrameLayout(this);loading.setBackgroundColor(0x99000000);loading.setClickable(true);
        ProgressBar spinner=new ProgressBar(this);spinner.setContentDescription("Wird geladen");
        loading.addView(spinner,new FrameLayout.LayoutParams(dp(40),dp(40),Gravity.CENTER));
        frame.addView(loading,new FrameLayout.LayoutParams(-1,-1));
    }
    private void message(String value) { loading(false);if(value==null||!value.isEmpty())toast(value==null?"Verbindung fehlgeschlagen.":value); }
    private void toast(String value) { if(value!=null && !value.isEmpty())Toast.makeText(this,value,Toast.LENGTH_LONG).show(); }
    private interface Work { void run() throws Exception; }
    private void background(Work work) { int started=generation;worker.execute(()->{try {work.run();} catch(Exception e) {ui(()->{if(generation==started)message(e.getMessage());});}}); }
    private void ui(Runnable action) { runOnUiThread(()->{if(!isDestroyed()) action.run();}); }
    private Api api() throws Exception { JSONObject c = vault.get("credentials"); if(c==null) throw new Exception("Bitte anmelden"); return new Api(c.getString("base"),c.getString("token")); }
    private JSONArray array(String key) { return snapshot == null ? new JSONArray() : snapshot.optJSONArray(key) == null ? new JSONArray() : snapshot.optJSONArray(key); }
    private JSONObject find(String key,String id) { JSONArray values=array(key); for(int i=0;i<values.length();i++){JSONObject v=values.optJSONObject(i);if(v!=null && v.optString("id").equals(id))return v;} return null; }
    private void login() {
        screen("Anmeldung");
        EditText endpoint=input("Factory-Adresse (HTTPS)","",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_URI);
        EditText key=input("Zugangsschlüssel","",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_PASSWORD);
        key.setSaveEnabled(false);
        Button signIn=FactoryStyle.button(this,"Anmelden",true);addControl(page,signIn,false);
        signIn.setOnClickListener(v->{
            String base=endpoint.getText().toString(), secret=key.getText().toString().trim();
            if(base.trim().isEmpty()){endpoint.setError("Factory-Adresse eingeben");return;}
            if(secret.isEmpty()){key.setError("Zugangsschlüssel eingeben");return;}
            int started=generation;
            signIn.setEnabled(false); loading(true);
            worker.execute(()->{
                try {
                    Api candidate=new Api(base,secret); JSONObject pending=vault.get("pending");
                    if(pending!=null && !pending.getString("base").equals(candidate.base)){
                        ui(()->loginFailed(started,signIn,"Ein ausstehender Auftrag gehört zu einer anderen Factory."));
                        return;
                    }
                    JSONObject fresh=candidate.request("GET","/v1/snapshot",null);
                    JSONObject previous=vault.get("credentials");
                    boolean newFactory=previous==null||!candidate.base.equals(previous.optString("base"));
                    InboxPush.login(this,new JSONObject().put("base",candidate.base).put("token",secret));
                    ui(()->{if(generation==started){if(newFactory){projectId=null;section=Section.OVERVIEW;}refresh(fresh);}});
                } catch(Api.HttpFailure e) {
                    android.util.Log.w("YoloForge","Factory sign-in failed: HTTP "+e.status);
                    ui(()->loginFailed(started,signIn,e.status==401 || e.status==403
                        ?"Zugang abgelehnt. Prüfe den Zugangsschlüssel."
                        :"Anmeldung fehlgeschlagen. Prüfe die Factory-Adresse."));
                } catch(Exception e) {
                    android.util.Log.w("YoloForge","Factory sign-in failed: "+e.getClass().getSimpleName());
                    ui(()->loginFailed(started,signIn,"Anmeldung fehlgeschlagen. Prüfe die Factory-Adresse und Verbindung."));
                }
            });
        });
    }
    private void loginFailed(int started, Button signIn, String reason) {
        if(generation!=started)return;
        signIn.setEnabled(true); message("");
        android.widget.Toast.makeText(this,reason,android.widget.Toast.LENGTH_LONG).show();
    }
    private void refresh() { refresh(null); }
    private void refresh(JSONObject verified) {
        if(frame==null)screen("Verbinden");
        loading(true);
        int started=generation;
        background(()->{JSONObject fresh;
            try{fresh=verified!=null?verified:api().request("GET","/v1/snapshot",null);}
            catch(Exception failure){
                android.util.Log.w("YoloForge","Factory refresh failed: "+failure.getClass().getSimpleName());
                ui(()->{if(generation!=started)return;setOnline(false);
                    if(snapshot==null)login();else if(showQuestions){showQuestions=false;questions();}else showSection();
                    toast("Factory nicht erreichbar. Bitte Verbindung prüfen.");
                });return;
            }
            JSONObject pending=vault.get("pending"); ui(()->{
            if(generation!=started)return;
             snapshot=fresh;setOnline(true);
            if(projectId!=null && find("projects",projectId)==null)projectId=null;
            if(projectId==null)try{
                JSONObject choice=vault.get("project-choice:"+api().base);
                if(choice!=null&&find("projects",choice.optString("id"))!=null)projectId=choice.optString("id");
            }catch(Exception ignored){}
            InboxPush.refresh(this,false,null);
            if(pending!=null){pendingScreen();return;}
            if(showQuestions){showQuestions=false;questions();return;}
            try{if(restoreBase!=null && !restoreBase.equals(api().base)){projectId=null;restoreCompose=false;archiveCommand=null;restoreQuestion=null;}}catch(Exception e){return;}restoreBase=null;
            if(deepQuestion!=null){JSONObject q=find("questions",deepQuestion);deepQuestion=null;if(q!=null && questionOpen(q)){String target=questionProject(q);projectId=target.isEmpty()?null:target;inboxTab=0;answer(q);}else questions();return;}
            if(deepProblem!=null){JSONObject p=activeProblem(deepProblem);deepProblem=null;if(p!=null){JSONObject t=find("tickets",p.optString("ticket_id"));projectId=t==null?null:t.optString("project_id");inboxTab=0;problem(p);}else questions();return;}
            if(restoreQuestion!=null){JSONObject q=find("questions",restoreQuestion);restoreQuestion=null;if(q!=null){answer(q);return;}}
            if(restoreCompose && projectId!=null && find("projects",projectId)!=null){restoreCompose=false;compose();return;}
            restoreCompose=false;
            if(archiveCommand!=null){archive(archiveCommand,archiveAfter,archiveRevision);return;}
            showSection();
        });});
    }
    private void showSection(){
        switch(section){
            case INTENTS -> project();
            case INBOX -> questions();
            case SESSIONS -> sessionList(false);
            case ACTIVITY -> sessionList(true);
            case NOTIFICATIONS -> factoryNotifications();
            case INFRASTRUCTURE -> infrastructure();
            case DIAGNOSE -> diagnostics();
            case OVERVIEW -> overview();
        }
    }
    private void appSettings(){
        screen("App-Einstellungen");button("Zurueck",this::showSection);
        navigationRow(page,R.drawable.ic_bell,"Android-Benachrichtigungen",false,0,this::notificationSettings);
        navigationRow(page,R.drawable.ic_download_simple,"App-Updates",false,0,this::updates);
        navigationRow(page,R.drawable.ic_sign_out,"Anmeldung wechseln",false,0,this::login);
    }
    private void notificationSettings(){
        screen("Android-Benachrichtigungen");button("Zurueck",this::appSettings);
        try{text("Factory: "+android.net.Uri.parse(api().base).getAuthority());}catch(Exception ignored){}
        text("Inbox-Meldungen öffnen offene Rückfragen und Probleme direkt. Wischst du eine Meldung weg, bleibt der Eintrag in der Inbox.");
        button("Benachrichtigungen einschalten",()->requestNotifications(10));
        button("Benachrichtigungen ausschalten",()->{try{
            InboxPush.disable(this);stopService(new Intent(this,EventService.class));
            toast("Benachrichtigungen ausgeschaltet.");
        }catch(Exception e){toast("Benachrichtigungen konnten nicht ausgeschaltet werden.");}});
        button("Agentenstatus anzeigen",()->requestNotifications(13));
        button("Agentenstatus ausblenden",()->stopService(new Intent(this,EventService.class)));
        button("Hintergrund-Push (ntfy)",this::pushSettings);
    }
    private void requestNotifications(int action){
        if(android.os.Build.VERSION.SDK_INT>=33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=android.content.pm.PackageManager.PERMISSION_GRANTED)
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},action);
        else startNotifications(action);
    }
    private void startNotifications(int action){
        if(!InboxPush.permitted(this)){toast("Android-Benachrichtigungen sind nicht erlaubt.");return;}
        if(action==13){startForegroundService(new Intent(this,EventService.class).putExtra("status_only",true));return;}
        InboxPush.refresh(this,true,result->ui(()->{
            if(InboxPush.UNCONFIGURED.equals(result)){
                startForegroundService(new Intent(this,EventService.class).putExtra("questions",true));
                toast("Kein Google-Push eingerichtet. Inbox-Empfang läuft nur über die Liveverbindung.");
            }else toast(result);
        }));
    }
    private void pushSettings(){
        screen("Hintergrund-Push");button("Zurueck",this::notificationSettings);
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
        screen("App-Updates");button("Zurueck",this::appSettings);loading(true);int started=generation;
        background(()->{JSONObject release=Updates.latest();ui(()->{if(generation!=started)return;loading(false);text("Release "+release.optString("tag"));
            button("Update herunterladen und installieren",()->{
                if(!getPackageManager().canRequestPackageInstalls()){startActivity(new Intent(android.provider.Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,android.net.Uri.parse("package:"+getPackageName())));message("Installation erlauben und danach erneut waehlen");return;}
                loading(true);background(()->{java.io.File file=Updates.fetch(this,release);ui(()->{if(generation!=started)return;loading(false);try{
                    Updates.verify(this,file);android.net.Uri uri=android.net.Uri.parse("content://"+BuildConfig.APPLICATION_ID+".updates/release-update.apk");startActivity(new Intent(Intent.ACTION_VIEW).setDataAndType(uri,"application/vnd.android.package-archive").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION));
                }catch(Exception e){message(e.getMessage());}});});
            });
        });});
    }
    @Override public void onRequestPermissionsResult(int request,String[] permissions,int[] results){super.onRequestPermissionsResult(request,permissions,results);if((request==10||request==13) && results.length>0 && results[0]==android.content.pm.PackageManager.PERMISSION_GRANTED)startNotifications(request);}
    private void project() {
        section=Section.INTENTS;screen("Intents");
        JSONObject p=find("projects",projectId); if(p==null){text("Wähle im Menü ein Projekt aus.");return;}
        EditText search=input("Intents durchsuchen …","",InputType.TYPE_CLASS_TEXT);
        LinearLayout tabs=new LinearLayout(this);addTabs(tabs,0);
        LinearLayout list=new LinearLayout(this);list.setOrientation(LinearLayout.VERTICAL);page.addView(list);
        String[] titles={"Queue","Entwurf","Archiv"};
        int[] selected={0};
        Runnable render=()->{
        list.removeAllViews();
        for(int i=0;i<tabs.getChildCount();i++)tabs.getChildAt(i).setBackground(interactive(this,i==selected[0]?SELECTED:0,4));
        String query=search.getText().toString().strip().toLowerCase(Locale.ROOT);
        JSONArray tasks=array("tickets");for(int i=0;i<tasks.length();i++){JSONObject t=tasks.optJSONObject(i);if(t==null || !t.optString("project_id").equals(projectId))continue;
            JSONObject c=find("commands",t.optString("execution_command"));String detail=c==null?t.optString("routing_note","Wartet"):c.optString("state");
            String state=t.optString("status");
            int bucket=t.optBoolean("draft")?1:Arrays.asList("done","cancelled").contains(state)?2:0;
            if(bucket!=selected[0] || !(t.optString("key")+" "+taskLabel(t,c)+" "+t.optString("prompt")).toLowerCase(Locale.ROOT).contains(query))continue;
            String badge=t.optBoolean("draft")?"Entwurf":"blocked".equals(state)?"Blockiert":"done".equals(state)?"Fertig":"in_progress".equals(state)?"In Arbeit":"Offen";
            int color="blocked".equals(state)?RED:"done".equals(state)?GREEN:"in_progress".equals(state)?BLUE:MUTED;
            inboxEntry(list,R.drawable.ic_list_checks,t.optString("key"),taskLabel(t,c),detail,badge,color,()->intentDetails(t,c,detail));
        }
        if(list.getChildCount()==0)emptyList(list,query.isEmpty()?"Keine Intents in dieser Ansicht.":"Keine passenden Intents.");
        };
        for(int i=0;i<titles.length;i++){
            int index=i;Button tab=FactoryStyle.button(this,titles[i],false);
            tabs.addView(tab,new LinearLayout.LayoutParams(0,-2,1));
            tab.setOnClickListener(v->{selected[0]=index;render.run();});
        }
        onSearchChanged(search,render);render.run();
    }
    private void onSearchChanged(EditText search,Runnable render){
        search.addTextChangedListener(new android.text.TextWatcher(){
            @Override public void beforeTextChanged(CharSequence s,int start,int count,int after){}
            @Override public void onTextChanged(CharSequence s,int start,int before,int count){render.run();}
            @Override public void afterTextChanged(android.text.Editable s){}
        });
    }
    private void intentDetails(JSONObject ticket,JSONObject command,String detail){
        screen("Intent");button("Zurück",this::showSection);text(ticket.optString("key"));
        sectionLabel(page,"Prompt");
        String original=intentPrompt(ticket);
        TextView promptView=label(this,original.isBlank()?"Kein Prompt hinterlegt.":original,15,original.isBlank()?MUTED:TEXT);
        promptView.setTypeface(android.graphics.Typeface.MONOSPACE);
        promptView.setTextIsSelectable(true);promptView.setPadding(dp(12),dp(10),dp(12),dp(10));
        promptView.setBackground(fill(this,CELL,4));addControl(page,promptView,true);
        JSONObject assigned=command==null?find("commands",ticket.optString("requested_session","")):command;
        JSONObject spec=assigned==null?null:assigned.optJSONObject("agent_spec");
        String model=intentModel(spec==null?null:spec.optJSONObject("profile"),"provider_id");
        TextView modelLabel=label(this,"Modell: "+(model.isEmpty()?"Default":model),15,TEXT);
        page.addView(modelLabel);
        JSONObject runner=assigned==null?null:find("runners",assigned.optString("runner_id"));
        text("Runner: "+(assigned==null?"Auto Runner":runner==null?"Nicht verfügbar":runner.optString("name","Runner")));
        text(detail);
        Runnable back=()->intentDetails(ticket,command,detail);
        Set<String> linked=new HashSet<>();
        if(isAgentRun(assigned)){
            linked.add(assigned.optString("id"));
            button("OpenCode anzeigen",()->session(assigned,back));
        }
        sectionLabel(page,"Verlauf");
        LinearLayout timeline=new LinearLayout(this);timeline.setOrientation(LinearLayout.VERTICAL);page.addView(timeline);
        String id=ticket.optString("id");
        loadIntentHistory(id,0,timeline,back,linked);
        if(online&&command==null){
            int started=generation;
            worker.execute(()->{
                try{
                    JSONObject models=api().optionalObject("/v1/tasks/"+id+"/models");
                    String selection=intentModel(models==null?null:models.optJSONObject("implementer"),"access_id");
                    if(!selection.isEmpty())ui(()->{if(generation==started)modelLabel.setText("Modell: "+selection);});
                }catch(Exception failure){
                    android.util.Log.w("YoloForge","Intent model read failed",failure);
                }
            });
        }
    }
    static String intentPrompt(JSONObject ticket){
        String prompt=ticket.isNull("prompt")?"":ticket.optString("prompt","");
        if(!prompt.isBlank())return prompt;
        String description=ticket.optString("description","");
        return description.isBlank()?ticket.optString("title",""):description;
    }
    static String intentModel(JSONObject selection,String providerKey){
        if(selection==null)return "";
        String model=selection.optString("model_id",""),provider=selection.optString(providerKey,"");
        return model.isEmpty()||provider.isEmpty()?model:model+" / "+provider;
    }
    private static boolean isAgentRun(JSONObject command){
        return command!=null&&"run_agent".equals(command.optString("kind"));
    }
    private void loadIntentHistory(String id,long before,LinearLayout timeline,Runnable back,Set<String> linked){
        Button more=FactoryStyle.button(this,before==0?"Verlauf laden":"Ältere Einträge laden",false);
        addControl(timeline,more,false);
        more.setOnClickListener(v->{
            more.setEnabled(false);
            int started=generation;
            worker.execute(()->{
                try{
                    JSONObject result=api().request("GET","/v1/tasks/"+id+"/history"+(before==0?"":"?before="+before),null);
                    List<JSONObject> entries=records(result.getJSONArray("entries"));
                    long next=result.optLong("next_before");
                    ui(()->{
                        if(generation!=started)return;
                        timeline.removeView(more);
                        if(before==0&&entries.isEmpty())emptyList(timeline,"Noch keine Einträge.");
                        for(JSONObject entry:entries){
                            timeline.addView(label(this,date(entry.optLong("created_unix_ms")),12,MUTED));
                            int color=switch(entry.optString("level")){
                                case "error" -> RED;
                                case "warning" -> AMBER;
                                case "success" -> GREEN;
                                default -> TEXT;
                            };
                            TextView message=label(this,entry.optString("message"),15,color);
                            message.setTextIsSelectable(true);message.setPadding(0,0,0,dp(8));timeline.addView(message);
                            JSONObject run=find("commands",entry.optString("command_id",""));
                            if(isAgentRun(run)&&linked.add(run.optString("id"))){
                                Button open=FactoryStyle.button(this,"OpenCode anzeigen",false);
                                addControl(timeline,open,false);open.setOnClickListener(link->session(run,back));
                            }
                        }
                        if(next>0&&(before==0||next<before))loadIntentHistory(id,next,timeline,back,linked);
                    });
                }catch(Exception failure){
                    android.util.Log.w("YoloForge","Intent history read failed",failure);
                    ui(()->{if(generation==started){more.setEnabled(true);toast("Verlauf konnte nicht geladen werden.");}});
                }
            });
        });
        if(before==0&&online)more.performClick();
    }
    private static String role(JSONObject command){
        JSONObject spec=command==null?null:command.optJSONObject("agent_spec");
        return spec==null?"":spec.optString("role");
    }
    private static String sessionRoot(JSONObject command){
        JSONObject spec=command.optJSONObject("agent_spec");
        JSONObject continuation=spec==null?null:spec.optJSONObject("continuation");
        return continuation==null?command.optString("id"):continuation.optString("root_command_id",command.optString("id"));
    }
    private String sessionTitle(JSONObject command){
        JSONObject progress=command.optJSONObject("agent_progress"), titles=snapshot.optJSONObject("session_titles");
        if(titles!=null){
            String id=progress==null?"":progress.optString("session_id");
            if(id.isEmpty())id="ses_factory_"+sessionRoot(command).replace("-","");
            String title=titles.optString(id);
            if(!title.isEmpty())return title;
        }
        JSONObject stream=find("streams",command.optString("stream_id"));
        return stream==null?"OpenCode":stream.optString("goal","OpenCode");
    }
    private boolean sessionClosed(JSONObject command){
        for(JSONObject closure:records(array("session_closures")))
            if(sessionRoot(command).equals(closure.optString("root_command_id")))return true;
        for(JSONObject direct:records(array("directs")))
            if(command.optString("id").equals(direct.optString("current_command_id")))return "archived".equals(direct.optString("state"));
        return false;
    }
    private static List<JSONObject> records(JSONArray input){
        List<JSONObject> list=new ArrayList<>();
        if(input!=null)for(int i=0;i<input.length();i++){JSONObject obj=input.optJSONObject(i);if(obj!=null)list.add(obj);}
        return list;
    }
    private List<JSONObject> projectSessions(boolean activity){
        Map<String,JSONObject> latest=new HashMap<>();
        for(JSONObject command:records(array("commands"))){
            if(!"run_agent".equals(command.optString("kind")))continue;
            JSONObject stream=find("streams",command.optString("stream_id"));
            if(stream==null||!stream.optString("project_id").equals(projectId))continue;
            String role=role(command);
            boolean supervisor="supervisor".equals(role),memory="memorizer".equals(role)||"orchestrator-memory".equals(role);
            JSONObject spec=command.optJSONObject("agent_spec");
            if(activity?(!supervisor&&!memory||activityRole==1&&!supervisor||activityRole==2&&!memory)
                :("merger".equals(role)||"direct".equals(role)||spec!=null&&spec.optBoolean("read_only")))continue;
            String key=sessionRoot(command);
            JSONObject previous=latest.get(key);
            if(previous==null||previous.optLong("created_unix_ms")<=command.optLong("created_unix_ms"))latest.put(key,command);
        }
        if(!activity)for(JSONObject direct:records(array("directs"))){
            if(!direct.optString("project_id").equals(projectId))continue;
            JSONObject command=find("commands",direct.optString("current_command_id"));
            if(command!=null)latest.put(sessionRoot(command),command);
        }
        List<JSONObject> result=new ArrayList<>(latest.values());
        result.sort(Comparator.comparingLong((JSONObject c)->c.optLong("created_unix_ms")).reversed());
        return result;
    }
    private void sessionList(boolean activity){
        section=activity?Section.ACTIVITY:Section.SESSIONS;screen(activity?"Projektaktivität":"Sessions");
        JSONObject project=find("projects",projectId);
        if(project==null){text("Wähle im Menü ein Projekt aus.");return;}
        if(!activity){
            LinearLayout header=(LinearLayout)surface.getChildAt(0);
            ImageButton actions=iconButton(header,R.drawable.ic_dots_three,"Weitere Aktionen",null);
            header.removeViewAt(header.getChildCount()-2);
            actions.setOnClickListener(v->{
                PopupMenu menu=new PopupMenu(this,actions);
                menu.getMenu().add("Aktualisieren").setOnMenuItemClickListener(item->{refresh();return true;});
                menu.getMenu().add("Neue manuelle Session").setIcon(R.drawable.ic_plus).setOnMenuItemClickListener(item->{
                    try{write("/v1/directs",new JSONObject().put("request_id",UUID.randomUUID().toString()).put("project_id",projectId));}
                    catch(Exception e){toast("Manuelle Session konnte nicht angelegt werden.");}
                    return true;
                });
                if(android.os.Build.VERSION.SDK_INT>=29)menu.setForceShowIcon(true);
                menu.show();
            });
        }
        EditText search=null;
        if(activity){
            LinearLayout controls=new LinearLayout(this);page.addView(controls);
            search=new EditText(this);search.setSingleLine(true);
            search.setText(activityQuery);FactoryStyle.input(search,"Projektaktivität durchsuchen …");
            controls.addView(search,new LinearLayout.LayoutParams(0,-2,1));
            ImageButton filter=iconButton(controls,R.drawable.ic_funnel_simple,"Rollen filtern",null);
            filter.setBackground(interactive(this,activityRole!=0?SELECTED:0,4));
            filter.setOnClickListener(v->{
                PopupMenu filters=new PopupMenu(this,filter);String[] titles={"Alle Rollen","Supervisor","Memorizer"};
                for(int i=0;i<titles.length;i++)filters.getMenu().add(1,i,i,titles[i]).setCheckable(true).setChecked(i==activityRole);
                filters.getMenu().setGroupCheckable(1,true,true);
                filters.setOnMenuItemClickListener(item->{activityRole=item.getItemId();sessionList(true);return true;});filters.show();
            });
        }
        List<JSONObject> sessions=projectSessions(activity);
        int closed=0;for(JSONObject command:sessions)if(sessionClosed(command))closed++;
        LinearLayout tabs=new LinearLayout(this);addTabs(tabs,0);
        for(int i=0;i<2;i++){
            boolean archive=i==1;
            Button tab=FactoryStyle.button(this,(archive?"Geschlossen ("+closed:"Offen ("+(sessions.size()-closed))+")",false);
            tab.setSelected(sessionArchive==archive);
            tab.setBackground(interactive(this,sessionArchive==archive?SELECTED:0,4));
            tabs.addView(tab,new LinearLayout.LayoutParams(0,dp(44),1));
            tab.setOnClickListener(v->{sessionArchive=archive;sessionList(activity);});
        }
        LinearLayout list=new LinearLayout(this);list.setOrientation(LinearLayout.VERTICAL);page.addView(list);
        Runnable render=()->{
        list.removeAllViews();boolean empty=true;
        String query=activity?activityQuery.trim().toLowerCase(Locale.ROOT):"";
        for(JSONObject command:sessions){
            if(sessionArchive!=sessionClosed(command))continue;
            String state=command.optString("state"), role=role(command);
            String badge=sessionArchive?"Geschlossen":"failed".equals(state)?"Fehlgeschlagen":"accepted".equals(state)?"Läuft":"pending".equals(state)?"Wartet":"Offen";
            int color="failed".equals(state)?RED:"accepted".equals(state)?BLUE:MUTED;
            String title=sessionTitle(command);
            if(!(title+" "+role).toLowerCase(Locale.ROOT).contains(query))continue;
            empty=false;
            inboxEntry(list,activity?R.drawable.ic_pulse:R.drawable.ic_terminal_window,"",title,
                activity?("supervisor".equals(role)?"Supervisor":"Memorizer"):"direct".equals(role)?"Direct":"OpenCode",
                badge,color,()->session(command));
        }
        if(empty)emptyList(list,activity?"Keine Einträge zur Projektaktivität.":"Keine Sessions in dieser Ansicht.");
        };
        if(search!=null){EditText field=search;onSearchChanged(field,()->{activityQuery=field.getText().toString();render.run();});}
        render.run();
    }
    private String activityQuery="";
    private static final long OVERVIEW_COMPLETED_MS=120_000;
    private Runnable overviewExpiry;
    private void cancelOverviewExpiry(){
        if(overviewExpiry!=null){connectionHandler.removeCallbacks(overviewExpiry);overviewExpiry=null;}
    }
    private void overview(){
        section=Section.OVERVIEW;screen("Overview");
        JSONObject project=find("projects",projectId);
        CheckBox only=new CheckBox(this);only.setText("Nur aktuelles Projekt");
        only.setEnabled(project!=null);only.setChecked(overviewOnlyProject&&project!=null);
        page.addView(only);
        LinearLayout stages=new LinearLayout(this);boolean wide=getResources().getConfiguration().screenWidthDp>=1000;
        stages.setOrientation(wide?LinearLayout.HORIZONTAL:LinearLayout.VERTICAL);page.addView(stages);
        only.setOnCheckedChangeListener((button,checked)->{overviewOnlyProject=checked;renderStages(stages,wide);});
        renderStages(stages,wide);
    }
    private void renderStages(LinearLayout stages,boolean wide){
        cancelOverviewExpiry();
        stages.removeAllViews();
        String[] titles={"Eingang","Verteilung","Implementierung","Review","Mergen","Erledigt"};
        List<List<JSONObject>> lanes=new ArrayList<>();
        for(String title:titles)lanes.add(new ArrayList<>());
        long now=System.currentTimeMillis(), nextExpiry=Long.MAX_VALUE;
        for(JSONObject t:records(array("tickets"))){
            if(find("projects",t.optString("project_id"))==null)continue;
            if(overviewOnlyProject&&projectId!=null&&!projectId.equals(t.optString("project_id")))continue;
            JSONObject command=find("commands",t.optString("execution_command"));
            int stage=overviewStage(t,command,now);
            if(stage>=0)lanes.get(stage).add(t);
            if(stage==5)nextExpiry=Math.min(nextExpiry,t.optLong("updated_unix_ms")+OVERVIEW_COMPLETED_MS);
        }
        if(nextExpiry!=Long.MAX_VALUE){
            overviewExpiry=()->renderStages(stages,wide);
            connectionHandler.postDelayed(overviewExpiry,Math.max(0,nextExpiry-System.currentTimeMillis()));
        }
        for(int i=0;i<titles.length;i++){
            List<JSONObject> lane=lanes.get(i);
            LinearLayout column=new LinearLayout(this);column.setOrientation(LinearLayout.VERTICAL);
            LinearLayout.LayoutParams layout=wide?new LinearLayout.LayoutParams(0,-2,1):new LinearLayout.LayoutParams(-1,-2);
            if(wide&&i>0)layout.setMarginStart(dp(8));
            stages.addView(column,layout);
            sectionLabel(column,titles[i]+" · "+lane.size());
            for(JSONObject ticket:lane){
                JSONObject command=find("commands",ticket.optString("execution_command"));
                String state=ticket.optString("status");
                String badge="blocked".equals(state)?"Blockiert":"done".equals(state)?"Erledigt":"Offen";
                int color="blocked".equals(state)?RED:"done".equals(state)?GREEN:BLUE;
                inboxEntry(column,R.drawable.ic_list_checks,ticket.optString("key"),taskLabel(ticket,command),"",badge,color,
                    ()->intentDetails(ticket,command,ticket.optString("routing_note")));
            }
        }
    }
    static int overviewStage(JSONObject ticket,JSONObject command,long now){
        String status=ticket.optString("status");
        if(ticket.optBoolean("draft")||"cancelled".equals(status))return -1;
        if("done".equals(status)){
            long updated=ticket.optLong("updated_unix_ms");
            return updated>0&&updated<=now&&now-updated<OVERVIEW_COMPLETED_MS?5:-1;
        }
        if("backlog".equals(status)&&command==null)return 0;
        String role=role(command),kind=command==null?"":command.optString("kind");
        if("merger".equals(role)||"integrate_review".equals(kind))return 4;
        if("reviewer".equals(role)||"review_checkpoint".equals(kind)||"validating".equals(status)||"review".equals(status))return 3;
        if("ready".equals(status)||"rework_queued".equals(status)||"orchestrator".equals(role)||"prepare_workspace".equals(kind))return 1;
        return command!=null||"in_progress".equals(status)?2:1;
    }
    private void factoryNotifications(){
        section=Section.NOTIFICATIONS;screen("Benachrichtigungen");notificationPage(0);
    }
    private void markAllNotificationsRead(){
        int started=generation;loading(true);
        worker.execute(()->{
            try{
                Api client=api();long before=0;
                List<JSONArray> batches=new ArrayList<>();
                do{
                    JSONObject result=client.request("GET","/v1/notifications"+(before==0?"":"?before="+before),null);
                    JSONArray ids=new JSONArray();
                    for(JSONObject entry:records(result.optJSONArray("entries")))if(!entry.optBoolean("read"))ids.put(entry.getLong("id"));
                    if(ids.length()>0)batches.add(ids);
                    before=result.optLong("next_before");
                }while(before>0);
                for(JSONArray ids:batches)client.request("POST","/v1/notifications/read",new JSONObject()
                    .put("request_id",UUID.randomUUID().toString()).put("ids",ids).put("read",true));
                ui(()->{if(generation==started)factoryNotifications();});
            }catch(Exception e){
                android.util.Log.w("YoloForge","Mark notifications failed",e);
                ui(()->{if(generation==started){factoryNotifications();toast("Benachrichtigungen konnten nicht als gelesen markiert werden.");}});
            }
        });
    }
    private void notificationPage(long before){
        loadPage("/v1/notifications"+(before==0?"":"?before="+before),"Benachrichtigungen",result->{
            unreadNotifications=result.optLong("unread");
            List<JSONObject> notices=records(result.optJSONArray("entries"));
            if(notices.isEmpty())emptyList(page,"Keine Benachrichtigungen.");
            for(JSONObject item:notices){
                JSONObject project=find("projects",item.optString("project_id"));
                boolean unread=!item.optBoolean("read");
                String context=(project==null?"Factory":project.optString("name"))+" · "+date(item.optLong("created_unix_ms"));
                inboxEntry(page,R.drawable.ic_bell,"",item.optString("title","Benachrichtigung"),context,
                    unread?"Ungelesen":"Gelesen",unread?AMBER:MUTED,()->openFactoryNotice(item));
            }
            older(result.optLong("next_before"),this::notificationPage);
        });
    }
    private void openFactoryNotice(JSONObject item){
        if(!item.optBoolean("read")){
            worker.execute(()->{
                try{api().request("POST","/v1/notifications/read",new JSONObject().put("request_id",UUID.randomUUID().toString())
                        .put("ids",new JSONArray().put(item.getLong("id"))).put("read",true));
                    ui(()->unreadNotifications=Math.max(0,unreadNotifications-1));
                }catch(Exception ignored){ui(()->toast("Benachrichtigung konnte nicht als gelesen markiert werden."));}
            });
        }
        JSONObject question=find("questions",item.optString("question_id"));
        JSONObject ticket=find("tickets",item.optString("ticket_id"));
        if(question!=null){String target=questionProject(question);if(!target.isEmpty())projectId=target;answer(question);return;}
        if(ticket!=null){intentDetails(ticket,find("commands",ticket.optString("execution_command")),ticket.optString("routing_note"));return;}
        JSONObject command=find("commands",item.optString("command_id"));
        if(command!=null&&"run_agent".equals(command.optString("kind"))){session(command);return;}
        screen("Benachrichtigung");button("Zurueck",this::factoryNotifications);text(item.optString("title"));text(date(item.optLong("created_unix_ms")));
    }
    private void infrastructure(){
        section=Section.INFRASTRUCTURE;screen("Infrastructure");
        sectionLabel(page,"FACTORY");
        factoryConnectionBadge=inboxEntry(page,R.drawable.ic_factory,"","Factory",snapshot.optString("server_version"),
            online?"Online":"Offline",online?GREEN:MUTED,this::diagnostics);
        sectionLabel(page,"RUNNER");
        List<JSONObject> runners=records(array("runners"));
        if(runners.isEmpty())emptyList(page,"Keine Runner verbunden.");
        for(JSONObject runner:runners){
            boolean connected="online".equals(runner.optString("connection"));
            JSONObject opencode=runner.optJSONObject("opencode");
            String details=runner.optString("location")+" · "+runner.optString("version")+
                (opencode==null?"":" · OpenCode: "+opencode.optString("status"));
            inboxEntry(page,R.drawable.ic_cpu,"",runner.optString("name","Runner"),details,
                connected?"Online":"Offline",connected?GREEN:MUTED,()->{
                    screen("Runner");button("Zurueck",this::infrastructure);
                    text(runner.optString("name"));text(runner.optString("location"));text(runner.optString("version"));
                     if(opencode!=null)text("OpenCode · "+opencode.optString("status"));
                });
        }
        sectionLabel(page,"CLIENTS");
        inboxEntry(page,R.drawable.ic_squares_four,"","YoloForge Android",BuildConfig.VERSION_NAME,
            "Dieses Gerät",MUTED,this::appSettings);
        int started=generation;
        worker.execute(()->{
            try{
                List<JSONObject> clients=records(api().array("/v1/desktop/clients"));
                ui(()->{
                    if(generation!=started)return;
                    for(JSONObject client:clients){
                        if(client.optString("id").endsWith(":"+Api.CLIENT_ID))continue;
                        inboxEntry(page,R.drawable.ic_squares_four,"",client.optString("name","Client"),
                            client.optString("version"),"Aktiv",GREEN,()->toast(client.optString("ip","Keine Adresse verfügbar")));
                    }
                });
            }catch(Exception e){ui(()->{if(generation==started)toast("Aktive Clients konnten nicht geladen werden.");});}
        });
    }
    private void diagnostics(){
        section=Section.DIAGNOSE;screen("Diagnose");
        sectionLabel(page,"FACTORY-EREIGNISSE");eventPage(0);
    }
    private void eventPage(long before){
        loadPage("/v1/events/history"+(before==0?"":"?before="+before),"Diagnose",result->{
            List<JSONObject> events=records(result.optJSONArray("events"));if(events.isEmpty())emptyList(page,"Keine Ereignisse.");
            for(JSONObject event:events){
                String kind=event.optString("kind");
                inboxEntry(page,R.drawable.ic_chart_line,"",kind,date(event.optLong("created_unix_ms")),
                    "Details",MUTED,()->{screen("Ereignisdetails");button("Zurueck",this::diagnostics);
                        loadPage("/v1/events/"+event.optLong("sequence"),"Ereignisdetails",payload->text(payload.toString()));});
            }
            older(result.optLong("next_before"),this::eventPage);
        });
    }
    private void older(long before,java.util.function.LongConsumer load){
        if(before==0)return;
        Button more=FactoryStyle.button(this,"Ältere Einträge laden",false);addControl(page,more,false);
        more.setOnClickListener(v->{page.removeView(more);load.accept(before);});
    }
    private void loadPage(String endpoint,String name,java.util.function.Consumer<JSONObject> ready){
        loadPage(endpoint,name,false,ready);
    }
    private void loadPage(String endpoint,String name,boolean optional,java.util.function.Consumer<JSONObject> ready){
        int started=generation;
        loading(true);
        worker.execute(()->{
            try{JSONObject result=optional?api().optionalObject(endpoint):api().request("GET",endpoint,null);ui(()->{if(generation==started){loading(false);ready.accept(result);}});}
            catch(Exception failure){
                android.util.Log.w("YoloForge","Factory data read failed: "+failure.getClass().getSimpleName());
                ui(()->{if(generation==started){loading(false);toast(name+" konnten nicht geladen werden.");}});
            }
        });
    }
    private String date(long epoch){
        return epoch<=0?"":android.text.format.DateFormat.getDateFormat(this).format(new Date(epoch))+" "+
            android.text.format.DateFormat.getTimeFormat(this).format(new Date(epoch));
    }
    static String taskLabel(JSONObject task, JSONObject command) {
        if (!task.optBoolean("draft") && command != null
                && "run_agent".equals(command.optString("kind"))
                && "succeeded".equals(command.optString("state"))) {
            JSONObject spec = command.optJSONObject("agent_spec");
            JSONObject result = command.optJSONObject("result");
            JSONObject agent = result == null ? null : result.optJSONObject("agent");
            if ((spec == null || !spec.optBoolean("read_only")) && agent != null
                    && "succeeded".equals(agent.optString("phase")) && !agent.isNull("summary")) {
                String summary = agent.optString("summary").trim();
                if (!summary.isEmpty()) return summary;
            }
        }
        String prompt = task.isNull("prompt") ? "" : task.optString("prompt");
        return prompt.trim().isEmpty() ? task.optString("title") : prompt;
    }
    private void compose() {
        section=Section.INTENTS;
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
    private void session(JSONObject latest) {
        session(latest,this::showSection);
    }
    private void session(JSONObject latest,Runnable back) {
        screen("OpenCode");button("Zurück",back);
        text(sessionTitle(latest));
        String id=latest.optString("id");
        button("Vollständiges Archiv",()->archive(id,0,null));
        loadPage("/v1/commands/"+id+"/excerpt","OpenCode-Nachrichten",true,excerpt->{
            List<JSONObject> messages=records(excerpt==null?null:excerpt.optJSONArray("messages"));
            if(messages.isEmpty())text("Noch keine OpenCode-Nachrichten verfügbar.");
            if(excerpt!=null&&excerpt.optBoolean("older_omitted"))text("Ältere Nachrichten findest du im vollständigen Archiv.");
            for(JSONObject message:messages){
                sectionLabel(page,message.optString("kind"));
                String body=message.optString("text");
                text(body+(message.optBoolean("truncated")&&!body.endsWith("…")?"…":""));
            }
        });
    }
    private void archive(String id,long after,Long revision) {
        screen("Archiv");archiveCommand=id;archiveAfter=after;archiveRevision=revision;
        button("Zurueck",this::showSection);loading(true);int started=generation;
        background(()->{
            JSONObject result=api().request("GET","/v1/commands/"+id+"/archive?after="+after+(revision==null?"":"&through_revision="+revision),null);
            JSONObject state=result.optJSONObject("status");long frozen=revision==null?(state==null?0:state.optLong("revision")):revision;
            JSONArray entries=result.getJSONArray("entries");long next=result.getLong("next_after");
            if(entries.length()>0 && next<=after)throw new Exception("Archivcursor ist nicht fortgeschritten");
            JSONArray supplements;
            try{supplements=api().request("GET","/v1/commands/"+id+"/files?through_revision="+frozen,null).getJSONArray("files");}
            catch(Api.HttpFailure error){if(error.status!=404)throw error;supplements=new JSONArray();}
            if(supplements.length()>16)throw new Exception("Ungueltiger Archivdateiindex");
            final JSONArray blobs=supplements;
            ui(()->{if(generation!=started)return;archiveRevision=frozen;
                message("Archivstand "+frozen+(state!=null && state.optBoolean("final_scan_complete")?" · Scan abgeschlossen":" · noch unvollstaendig"));
                JSONObject termination=result.optJSONObject("termination");if(termination!=null && termination.optBoolean("archive_may_be_incomplete"))text("Die Runtime wurde mit moeglicherweise unvollstaendigem Archiv beendet.");
                for(int i=0;i<entries.length();i++){JSONObject entry=entries.optJSONObject(i);if(entry==null)continue;JSONObject msg=entry.optJSONObject("message");if(msg==null)continue;
                    text("Nachricht "+msg.optString("id")+" · Revision "+entry.optLong("sample_revision"));
                    try{text(ArchiveContent.render(msg));for(JSONObject attachment:ArchiveContent.files(msg)){
                        String label=attachment.optString("name","Anhang");if(!"captured".equals(attachment.optString("status"))){text(label+" · nicht verfuegbar");continue;}
                        button("Anhang speichern: "+label,()->chooseArchiveFile(attachment));
                    }}catch(Exception e){text("Archivinhalt ungueltig: "+e.getMessage());}
                    for(int j=0;j<blobs.length();j++){JSONObject blob=blobs.optJSONObject(j);if(blob==null || !blob.optString("message_id").equals(msg.optString("id")))continue;
                        try{JSONObject descriptor=new JSONObject().put("status","stored_blob").put("command_id",id).put("name",blob.optString("name")).put("blob",blob);
                            button("Archivdatei speichern: "+blob.optString("name")+" ("+blob.optLong("bytes")+" Bytes)",()->chooseArchiveFile(descriptor));
                        }catch(Exception e){text("Ungueltiger Dateiindex");}
                    }
                }
                if(entries.length()==0)text("Ende des Archivs");else button("Naechste Seite",()->archive(id,next,frozen));
                if(after>0)button("Zum Anfang",()->archive(id,0,frozen));
                button("Neuen Archivstand laden",()->archive(id,0,null));
            });
        });
    }
    private void chooseArchiveFile(JSONObject file){try{
        if("stored_blob".equals(file.optString("status")))ArchiveBlobs.validate(file);else ArchiveContent.bytes(file);
        vault.put("archive-export",new JSONObject().put("base",api().base).put("file",file));
        startActivityForResult(new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/octet-stream").putExtra(Intent.EXTRA_TITLE,ArchiveContent.safeName(file.optString("name"))),12);
    }catch(Exception e){message(e.getMessage());}}
    private void exportArchiveFile(android.net.Uri uri){background(()->{
        JSONObject pending=vault.get("archive-export");if(pending==null)throw new Exception("Kein Anhang zum Export gespeichert");
        if(!api().base.equals(pending.getString("base")))throw new Exception("Anhang gehoert zu einer anderen Factory");
        JSONObject file=pending.getJSONObject("file");
        if("stored_blob".equals(file.optString("status"))){
            java.io.File temporary=ArchiveBlobs.download(api(),file,getCacheDir());
            try(var in=new java.io.FileInputStream(temporary);var out=getContentResolver().openOutputStream(uri,"wt")){
                if(out==null)throw new Exception("Ausgabedatei nicht verfuegbar");byte[] buffer=new byte[65536];int n;while((n=in.read(buffer))!=-1)out.write(buffer,0,n);out.flush();
            }finally{temporary.delete();}
        }else{
            byte[] bytes=ArchiveContent.bytes(file);
            try(var out=getContentResolver().openOutputStream(uri,"wt")){if(out==null)throw new Exception("Ausgabedatei nicht verfuegbar");out.write(bytes);out.flush();}
        }
        vault.put("archive-export",null);ui(()->message("Anhang gespeichert und SHA256 geprueft"));
    });}
    private void questions() {
        section=Section.INBOX;
        screen("Inbox");
        LinearLayout toolbar=new LinearLayout(this);toolbar.setGravity(Gravity.CENTER_VERTICAL);page.addView(toolbar);
        EditText search=new EditText(this);search.setInputType(InputType.TYPE_CLASS_TEXT);search.setSingleLine(true);
        search.setContentDescription("Inbox durchsuchen");FactoryStyle.input(search,"Inbox durchsuchen …");
        search.setCompoundDrawablesRelativeWithIntrinsicBounds(R.drawable.ic_magnifying_glass,0,0,0);
        search.setCompoundDrawablePadding(dp(9));search.setText(inboxQuery);
        toolbar.addView(search,new LinearLayout.LayoutParams(0,-2,1));
        LinearLayout tabs=new LinearLayout(this);
        if(getResources().getConfiguration().screenWidthDp>=700) {
            LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(dp(245),-2);params.setMarginStart(dp(8));toolbar.addView(tabs,params);
        }
        iconButton(toolbar,R.drawable.ic_funnel_simple,"Inbox filtern",()->{});
        View filterButton=toolbar.getChildAt(toolbar.getChildCount()-1);
        filterButton.setSelected(inboxFilter!=0);
        if(inboxFilter!=0)filterButton.setBackground(interactive(this,SELECTED,4));
        if(tabs.getParent()==null){
            addTabs(tabs,10);
        }else{
            ((LinearLayout.LayoutParams)toolbar.getLayoutParams()).bottomMargin=dp(TAB_LIST_GAP);
        }
        View gap=new View(this);page.addView(gap,new LinearLayout.LayoutParams(-1,dp(14)));
        LinearLayout entries=new LinearLayout(this);entries.setOrientation(LinearLayout.VERTICAL);page.addView(entries,new LinearLayout.LayoutParams(-1,-2));
        filterButton.setOnClickListener(v->{
            PopupMenu menu=new PopupMenu(this,filterButton);
            String[] filters={"Alle","Fragen","Probleme"};
            for(int i=0;i<filters.length;i++)menu.getMenu().add(1,i,i,filters[i]).setCheckable(true).setChecked(i==inboxFilter);
            menu.getMenu().setGroupCheckable(1,true,true);
            menu.setOnMenuItemClickListener(item->{
                inboxFilter=item.getItemId();filterButton.setSelected(inboxFilter!=0);
                filterButton.setBackground(interactive(this,inboxFilter!=0?SELECTED:0x00000000,4));
                renderInbox(tabs,entries);
                return true;
            });menu.show();
        });
        search.addTextChangedListener(new android.text.TextWatcher(){
            @Override public void beforeTextChanged(CharSequence s,int start,int count,int after){}
            @Override public void onTextChanged(CharSequence s,int start,int before,int count){inboxQuery=s.toString();renderInbox(tabs,entries);}
            @Override public void afterTextChanged(android.text.Editable s){}
        });
        page.setFocusableInTouchMode(true);page.requestFocus();
        renderInbox(tabs,entries);
    }
    private void renderInbox(LinearLayout tabs,LinearLayout entries){
        tabs.removeAllViews();entries.removeAllViews();
        for(int i=0;i<2;i++){
            final int selected=i;
            boolean archive=i==1;
            String title=archive?"Archiv":"Offen";
            Button tab=FactoryStyle.button(this,title+" ("+inboxCount(archive,inboxFilter)+")",false);
            boolean active=inboxTab==i;
            tab.setSelected(active);tab.setTextColor(active?TEXT:MUTED);
            tab.setBackground(interactive(this,active?SELECTED:0x00000000,2));
            if(active){
                var stripe=new android.graphics.drawable.LayerDrawable(new android.graphics.drawable.Drawable[]{fill(this,BLUE,0)});
                stripe.setLayerHeight(0,dp(3));stripe.setLayerGravity(0,Gravity.BOTTOM|Gravity.FILL_HORIZONTAL);
                tab.setForeground(stripe);
            }
            LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(0,dp(44),1);
            if(i==0)params.setMarginEnd(dp(6));tabs.addView(tab,params);
            tab.setOnClickListener(v->{inboxTab=selected;renderInbox(tabs,entries);});
        }
        boolean archive=inboxTab==1;
        String query=inboxQuery.trim().toLowerCase(Locale.ROOT);
        if(inboxFilter!=1){
            JSONArray problems=array("intent_problems"); Set<String> seen=new HashSet<>();
            for(int i=0;i<problems.length();i++){
                JSONObject p=problems.optJSONObject(i);
                if(!problemVisible(p,archive)||!seen.add(p.optString("ticket_id")+":"+p.optString("command_id")))continue;
                JSONObject t=find("tickets",p.optString("ticket_id"));
                String key=t.optString("key"),title=taskLabel(t,null);
                if(!matchesQuery(query,title,key,p.optString("reason")))continue;
                inboxEntry(entries,R.drawable.ic_warning_circle,key,title,p.optString("reason"),
                    archive?"Archiv":"Handlungsbedarf",archive?MUTED:RED,()->problem(p));
            }
        }
        if(inboxFilter!=2){
            JSONArray questions=array("questions");
            for(int i=0;i<questions.length();i++){
                JSONObject q=questions.optJSONObject(i);if(!inboxQuestionVisible(q,archive))continue;
                JSONObject ext=q.optJSONObject("external"),payload=ext==null?null:ext.optJSONObject("payload");
                String title=payload==null?"Rückfrage":payload.optString("title",payload.optString("action","Rückfrage"));
                String project=questionProject(q);
                JSONObject record=find("projects",project);
                String projectName=record==null?"":record.optString("name");
                boolean permission=ext!=null&&"permission".equals(ext.optString("kind"));
                String kind=permission?"Berechtigungsanfrage":"Rückfrage";
                if(!matchesQuery(query,title,projectName,kind,payload==null?"":payload.toString()))continue;
                inboxEntry(entries,permission?R.drawable.ic_warning_circle:R.drawable.ic_chat_circle_text,"",title,
                    projectName.isEmpty()?kind:kind+" · "+projectName,
                    archive?("answered".equals(q.optString("state"))?"Beantwortet":"Archiv"):"Offen",
                    archive?("answered".equals(q.optString("state"))?GREEN:MUTED):AMBER,()->answer(q));
            }
        }
        if(entries.getChildCount()==0){
            emptyList(entries,query.isEmpty()?(archive?"Keine archivierten Einträge":"Keine offenen Einträge"):"Keine passenden Einträge");
        }
    }
    private static final int TAB_LIST_GAP=12;
    private void addTabs(LinearLayout tabs,int topMargin){
        LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(-1,-2);
        params.topMargin=dp(topMargin);
        params.bottomMargin=dp(TAB_LIST_GAP);
        page.addView(tabs,params);
    }
    private void emptyList(LinearLayout list,String message){
        TextView empty=label(this,message,15,MUTED);
        empty.setGravity(Gravity.CENTER);
        empty.setPaddingRelative(dp(16),dp(32),dp(16),dp(20));
        list.addView(empty,new LinearLayout.LayoutParams(-1,-2));
    }
    private TextView inboxEntry(LinearLayout list,int glyph,String key,String title,String details,String status,int color,Runnable action){
        LinearLayout card=new LinearLayout(this);card.setOrientation(LinearLayout.VERTICAL);
        card.setPaddingRelative(dp(14),dp(12),dp(14),dp(12));card.setBackground(interactive(this,CELL,2));
        LinearLayout top=new LinearLayout(this);top.setGravity(Gravity.CENTER_VERTICAL);card.addView(top);
        ImageView symbol=icon(glyph,color);
        LinearLayout.LayoutParams symbolParams=new LinearLayout.LayoutParams(dp(19),dp(19));symbolParams.setMarginEnd(dp(9));top.addView(symbol,symbolParams);
        if(!key.isEmpty()){
            TextView id=pill(this,key,MUTED);id.setMaxWidth(dp(95));
            LinearLayout.LayoutParams idParams=new LinearLayout.LayoutParams(-2,-2);idParams.setMarginEnd(dp(6));top.addView(id,idParams);
        }
        TextView titleView=label(this,title,16,TEXT);truncate(titleView);
        top.addView(titleView,new LinearLayout.LayoutParams(0,-2,1));
        LinearLayout.LayoutParams pillParams=new LinearLayout.LayoutParams(-2,-2);pillParams.setMarginStart(dp(6));
        TextView badge=pill(this,status,color);badge.setMaxWidth(dp(115));top.addView(badge,pillParams);
        if(!details.isEmpty()){
            TextView metadata=label(this,details,13,MUTED);
            truncate(metadata);metadata.setPaddingRelative(dp(28),dp(5),0,0);card.addView(metadata);
        }
        card.setContentDescription(title);card.setFocusable(true);card.setOnClickListener(v->action.run());addControl(list,card,true);
        return badge;
    }
    private static boolean matchesQuery(String search,String... values){
        if(search.isEmpty())return true;
        for(String v:values)if(v.toLowerCase(Locale.ROOT).contains(search))return true;
        return false;
    }
    private int inboxCount(boolean archive,int filter){
        int count=0;
        if(filter!=2){
            JSONArray questions=array("questions");
            for(int i=0;i<questions.length();i++)if(inboxQuestionVisible(questions.optJSONObject(i),archive))count++;
        }
        if(filter!=1){
            JSONArray problems=array("intent_problems"); Set<String> seen=new HashSet<>();
            for(int i=0;i<problems.length();i++){
                JSONObject p=problems.optJSONObject(i);
                if(problemVisible(p,archive)&&seen.add(p.optString("ticket_id")+":"+p.optString("command_id")))count++;
            }
        }
        return count;
    }
    private String questionProject(JSONObject question){
        JSONObject command=find("commands",question.optString("command_id"));
        if(command==null)return "";
        String streamId=command.optString("stream_id");
        JSONArray directs=array("directs");
        for(int i=0;i<directs.length();i++){
            JSONObject direct=directs.optJSONObject(i);
            if(direct!=null&&streamId.equals(direct.optString("stream_id")))return "";
        }
        JSONObject stream=find("streams",streamId);
        return stream==null?"":stream.optString("project_id");
    }
    private boolean inboxQuestionVisible(JSONObject q,boolean archive){
        if(q==null||"automation pending".equals(q.optString("orchestration_note")))return false;
        String project=questionProject(q);
        if(project.isEmpty()||find("projects",project)==null||(projectId!=null&&!projectId.equals(project)))return false;
        return archive!=questionOpen(q);
    }
    private boolean problemVisible(JSONObject p,boolean archive){
        if(p==null)return false;
        JSONObject ticket=find("tickets",p.optString("ticket_id"));
        return ticket!=null && find("projects",ticket.optString("project_id"))!=null
            && (projectId==null||projectId.equals(ticket.optString("project_id"))) && archive!=problemOpen(p);
    }
    static boolean questionOpen(JSONObject q){
        if(q==null)return false;
        String state=q.optString("state");
        return "rejected".equals(state) || "pending".equals(state) && !"automation pending".equals(q.optString("orchestration_note"));
    }
    private JSONObject activeProblem(String id){
        JSONArray problems=array("intent_problems");
        for(int i=0;i<problems.length();i++){JSONObject p=problems.optJSONObject(i);if(p!=null && id.equals(p.optString("ticket_id")) && problemOpen(p))return p;}
        return null;
    }
    private boolean problemOpen(JSONObject p){
        if(p==null)return false;JSONObject t=find("tickets",p.optString("ticket_id"));
        if(t==null || t.optBoolean("draft") || find("projects",t.optString("project_id"))==null)return false;
        String status=t.optString("status"), command=p.optString("command_id");
        String note=t.optString("routing_note");
        if("in_progress".equals(status) && ("Review-Snapshot wird automatisch erneut gesichert.".equals(note)
            || "Geprüftes Ergebnis wird automatisch erneut integriert.".equals(note)))return false;
        JSONArray resets=array("draft_resets");
        for(int i=0;i<resets.length();i++){
            JSONObject r=resets.optJSONObject(i);
            if(r!=null && p.optString("ticket_id").equals(r.optString("ticket_id")) && command.equals(r.optString("command_id")))return false;
        }
        return command.equals(t.optString("execution_command")) && ("blocked".equals(status)||"in_progress".equals(status))
            || "reassign_session".equals(p.optString("solution")) && ("backlog".equals(status)||"ready".equals(status))
                && t.isNull("execution_command") && command.equals(t.optString("requested_session"));
    }
    private void problem(JSONObject p){
        section=Section.INBOX;
        screen("Problem");button("Zurueck",this::questions);
        JSONObject ticket=find("tickets",p.optString("ticket_id"));
        if(ticket!=null)text(taskLabel(ticket,null));
        text(p.optString("reason"));
        if(ticket!=null)button("Projekt öffnen",()->{projectId=ticket.optString("project_id");project();});
    }
    private void answer(JSONObject q) {
        section=Section.INBOX;
        screen("Antwort");button("Zurueck",this::questions);text(q.optString("orchestration_note"));
        JSONObject ext=q.optJSONObject("external"),payload=ext==null?null:ext.optJSONObject("payload");if(payload==null)return;
        text(payload.optString("title",payload.optString("action")));text(payload.optString("description"));
        if(!questionOpen(q)){text("Diese Frage wurde bereits beantwortet oder geschlossen.");return;}
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
        write("POST",path,body);
    }
    private void write(String method,String path,JSONObject body)throws Exception {
        if(vault.get("pending")!=null){pendingScreen();return;}
        new Outbox(vault).prepare(api(),method,path,body);
        if(composing){vault.put(draftKey(),null);composing=false;}
        if(path.equals("/v1/projects")||path.startsWith("/v1/projects/"))closeProjectDropdown();
        else pendingScreen();
        retry();
    }
    private void pendingScreen(){screen("Auftrag gespeichert");text("Bei Verbindungsabbruch bleibt der identische Auftrag mit derselben Request-ID gespeichert. Erneutes Senden erzeugt keinen zweiten Auftrag.");button("Erneut senden",this::retry);button("Neu anmelden",this::login);}
    private void retry(){loading(true);background(()->{
        JSONObject pending=vault.get("pending");if(pending==null){ui(this::refresh);return;}Api client=api();if(!client.base.equals(pending.getString("base")))throw new Exception("Auftrag gehoert zu einer anderen Factory");
        String path=pending.getString("path"), method=pending.optString("method","POST");
        boolean create=path.equals("/v1/projects"), projectMutation=create||path.startsWith("/v1/projects/");
        JSONObject result;
        try{result=new Outbox(vault).send(client);}
        catch(Exception failure){
            if(!projectMutation){
                if(failure instanceof Api.HttpFailure e && Arrays.asList(400,403,404,409,422).contains(e.status)){
                    ui(()->{showSection();toast("Auftrag abgelehnt. Bitte aktualisieren und korrigieren.");});return;
                }
                throw failure;
            }
            android.util.Log.w("YoloForge","Project mutation failed: "+failure);
            boolean rejected=failure instanceof Api.HttpFailure e && Arrays.asList(400,403,404,409,422).contains(e.status);
            ui(()->{
                message("");
                if(rejected){showSection();toast("Projektänderung abgelehnt. Bitte Angaben oder Projektstatus prüfen.");}
                else {pendingScreen();toast("Projektänderung noch nicht bestätigt. Erneut senden ist möglich.");}
            });
            return;
        }
        ui(()->{
            if(create||"PATCH".equals(method)){
                saveProjectDraft=null;
                projectDrafts.remove(client.base+path);
            }
            if(create && result!=null){
                projectId=result.optString("id");
                try{vault.put("project-choice:"+client.base,new JSONObject().put("id",projectId));}
                catch(Exception e){toast("Projektauswahl konnte nicht gespeichert werden.");}
            }
            refresh();
            if(projectMutation&&!path.endsWith("/pause"))toast(create?"Projekt hinzugefügt.":"DELETE".equals(method)?"Projekt gelöscht.":"Projekt gespeichert.");
        });
    });}
}
