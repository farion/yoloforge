package dev.yoloforge.mobile;

import android.content.Context;
import android.graphics.Typeface;
import android.text.Html;
import android.text.SpannableString;
import android.text.TextUtils;
import android.text.style.ForegroundColorSpan;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.regex.Pattern;
import org.json.JSONArray;
import org.json.JSONObject;
import static dev.yoloforge.mobile.FactoryStyle.*;

/** Native, read-only rendering. Archive strings are never executed or loaded as URLs. */
final class OpenCodeTranscript {
    private static final Pattern CODE_TOKEN=Pattern.compile(
        "\"(?:\\\\.|[^\"\\\\])*\"|'(?:\\\\.|[^'\\\\])*'|//[^\\n]*|\\b(?:fn|let|mut|pub|struct|impl|class|new|return|const|function|if|else|import|def|true|false|null|self|this)\\b|\\b\\d+(?:\\.\\d+)?\\b");
    private final LinearLayout rows;
    private final Context context;
    private final List<JSONObject> tools=new ArrayList<>();

    OpenCodeTranscript(LinearLayout rows){this.rows=rows;context=rows.getContext();}

    void append(JSONObject message){
        String role=message.optString("type",message.optString("role",message.optString("kind")));
        String text=string(message,"text");
        JSONArray parts=message.optJSONArray("content");
        if(parts==null)parts=message.optJSONArray("parts");
        if("user".equals(role)){
            flushTools();
            StringBuilder body=new StringBuilder(text);
            if(parts!=null)for(int i=0;i<parts.length();i++){
                JSONObject part=parts.optJSONObject(i);
                if(part!=null&&"text".equals(part.optString("type"))&&!text.equals(string(part,"text")))
                    appendLine(body,string(part,"text"));
            }
            String content=string(message,"content");
            if(!content.equals(text))appendLine(body,content);
            if(message.optBoolean("truncated")&&!body.toString().endsWith("…"))body.append('…');
            if(body.length()>0)user(body.toString());
            return;
        }
        if(message.optBoolean("truncated")&&!text.endsWith("…"))text+="…";
        boolean answered=!text.isBlank();
        if(answered){flushTools();markdown(text);}
        String content=string(message,"content");
        if(!content.isBlank()&&!content.equals(text)){flushTools();markdown(content);answered=true;}
        if(parts!=null)for(int i=0;i<parts.length();i++){
            JSONObject part=parts.optJSONObject(i);
            if(part==null)continue;
            switch(part.optString("type")){
                case "tool","tool-call","tool_call","tool-result","tool_result" -> tools.add(part);
                case "text" -> {
                    String body=string(part,"text");
                    if(!body.isBlank()&&!body.equals(text)){flushTools();markdown(body);answered=true;}
                }
                default -> {}
            }
        }
        if(answered&&"assistant".equals(role)&&message.isNull("error")){
            String footer=footer(message);
            if(!footer.isEmpty()){flushTools();line(rows,footer,MUTED);}
        }
    }

    void finish(){flushTools();}

    private void user(String text){
        LinearLayout block=new LinearLayout(context);block.setBackgroundColor(CELL);
        View marker=new View(context);marker.setBackgroundColor(BLUE);
        block.addView(marker,new LinearLayout.LayoutParams(dp(6),-1));
        TextView prompt=mono("\n"+text+"\n",TEXT);
        prompt.setPadding(dp(12),0,dp(12),0);
        block.addView(prompt,new LinearLayout.LayoutParams(0,-2,1));
        addBlock(block);
    }

    private void markdown(String body){
        LinearLayout block=new LinearLayout(context);block.setOrientation(LinearLayout.VERTICAL);
        for(String piece:body.split("(?m)^ {0,3}```[^\\n]*\\n?",-1)){
            int index=block.getChildCount();
            // Empty pieces still consume their position in the alternating sequence.
            if(index%2==0){
                TextView prose=mono("",TEXT);
                if(!piece.isEmpty())prose.setText(Html.fromHtml(markdownHtml(piece.stripTrailing()),Html.FROM_HTML_MODE_LEGACY));
                else prose.setVisibility(View.GONE);
                block.addView(prose);
            }else{
                LinearLayout code=new LinearLayout(context);code.setOrientation(LinearLayout.VERTICAL);
                code.setBackgroundColor(CELL);
                code(code,piece.stripTrailing(),TEXT,true);
                block.addView(code);
            }
        }
        addBlock(block);
    }

    static String markdownHtml(String prose){
        String safe=escape(prose);
        return safe.replaceAll("(?m)^#{1,6}\\s+(.+)$","<b>$1</b>")
            .replaceAll("\\*\\*([^*\\n]+)\\*\\*","<b>$1</b>")
            .replaceAll("`([^`\\n]+)`","<tt>$1</tt>")
            .replaceAll("(?m)^[-*] ","• ").replace("\n","<br>");
    }

    private void code(LinearLayout target,String body,int base,boolean highlight){
        String[] lines=body.split("\n",-1);
        boolean patch=body.startsWith("diff --git")||body.startsWith("@@")||body.startsWith("*** Begin Patch")
            ||body.startsWith("Index: ")||body.startsWith("--- ");
        if(!patch){
            TextView row=line(target,body,base);
            if(highlight)row.setText(syntax(body,0));
            return;
        }
        for(int i=0;i<lines.length;i++){
            String value=lines[i];
            int color=value.startsWith("+")&&!value.startsWith("+++")?GREEN:
                value.startsWith("-")&&!value.startsWith("---")?RED:base;
            String numbered=String.format(Locale.ROOT,"%3d  %s",i+1,value);
            TextView row=line(target,numbered,color);
            row.setText(syntax(numbered,numbered.length()-value.length()));
            if(color!=base)row.setBackgroundColor(color==GREEN?0xff173329:0xff3b2429);
        }
    }
    private static CharSequence syntax(String code,int offset){
        SpannableString styled=new SpannableString(code);
        var matches=CODE_TOKEN.matcher(code).region(offset,code.length());
        while(matches.find()){
            char first=code.charAt(matches.start());
            int color=first=='/'?MUTED:first=='"'||first=='\''?GREEN:Character.isDigit(first)?AMBER:BLUE;
            styled.setSpan(new ForegroundColorSpan(color),matches.start(),matches.end(),0);
        }
        return styled;
    }

    private void flushTools(){
        if(tools.isEmpty())return;
        List<JSONObject> group=new ArrayList<>(tools);tools.clear();
        LinearLayout block=new LinearLayout(context);block.setOrientation(LinearLayout.VERTICAL);
        Button toggle=button(context,"+  Tool-Aktivität ("+group.size()+")",false);
        toggle.setGravity(android.view.Gravity.CENTER_VERTICAL|android.view.Gravity.START);
        block.addView(toggle,new LinearLayout.LayoutParams(-1,-2));
        LinearLayout details=new LinearLayout(context);details.setOrientation(LinearLayout.VERTICAL);
        details.setVisibility(View.GONE);block.addView(details);
        toggle.setOnClickListener(v->{
            boolean open=details.getVisibility()==View.VISIBLE;
            if(!open&&details.getChildCount()==0)for(JSONObject part:group)tool(details,part);
            details.setVisibility(open?View.GONE:View.VISIBLE);
            toggle.setText((open?"+":"−")+"  Tool-Aktivität ("+group.size()+")");
        });
        addBlock(block);
    }

    private void tool(LinearLayout target,JSONObject part){
        JSONObject state=part.optJSONObject("state");
        JSONObject input=state==null?null:state.optJSONObject("input");
        if(input==null)input=part.optJSONObject("input");
        String name=part.optString("name",part.optString("tool","Tool"));
        String argument="";
        if(input!=null)for(String key:new String[]{"command","filePath","path","pattern","query"}){
            argument=string(input,key);if(!argument.isEmpty())break;
        }
        LinearLayout block=new LinearLayout(context);block.setOrientation(LinearLayout.VERTICAL);block.setBackgroundColor(CELL);
        line(block,"\n$ "+name+(argument.isEmpty()?"":" "+argument),MUTED);
        Object output=state==null?null:state.opt("content");
        if(output==null||output==JSONObject.NULL)output=state==null?null:state.opt("output");
        if(output==null||output==JSONObject.NULL)output=part.opt("output");
        StringBuilder body=new StringBuilder();
        if(output instanceof String)body.append((String)output);
        else if(output instanceof JSONArray){
            JSONArray items=(JSONArray)output;
            for(int i=0;i<items.length();i++)appendLine(body,string(items.optJSONObject(i),"text"));
        }
        if(body.length()>0)code(block,body.toString(),MUTED,false);
        line(block," ",MUTED);
        target.addView(block);
    }

    private static String footer(JSONObject message){
        JSONObject time=message.optJSONObject("time");
        if(time==null||!time.has("completed")||"tool-calls".equals(message.optString("finish")))return "";
        double created=time.optDouble("created"),completed=time.optDouble("completed");
        if(!Double.isFinite(created)||!Double.isFinite(completed)||completed<created)return "";
        JSONObject model=message.optJSONObject("model"),tokens=message.optJSONObject("tokens");
        String name=string(model,"name");if(name.isEmpty())name=string(model,"id");
        String result=(name.isEmpty()?"":name+" · ")+String.format(Locale.ROOT,"%.1f s",(completed-created)/1000);
        double elapsed=time.optDouble("streamed",completed)-time.optDouble("firstToken",created);
        double output=tokens==null?0:tokens.optDouble("output",0);
        return elapsed>0&&output>0?result+String.format(Locale.ROOT," · %.1f tok/s",output*1000/elapsed):result;
    }

    private void addBlock(View view){
        if(rows.getChildCount()>0)line(rows," ",TEXT);
        rows.addView(view,new LinearLayout.LayoutParams(-1,-2));
    }
    private TextView line(LinearLayout parent,String value,int color){
        TextView view=mono(value,color);parent.addView(view,new LinearLayout.LayoutParams(-1,-2));return view;
    }
    private TextView mono(String value,int color){
        TextView view=label(context,value,14,color);view.setTypeface(Typeface.MONOSPACE);view.setIncludeFontPadding(false);
        view.setTextIsSelectable(true);return view;
    }
    private int dp(int value){return FactoryStyle.dp(context,value);}
    private static String escape(String value){return TextUtils.htmlEncode(value).replace("&#39;","'");}
    private static String string(JSONObject object,String key){
        Object value=object==null?null:object.opt(key);return value instanceof String?(String)value:"";
    }
    private static void appendLine(StringBuilder out,String value){if(!value.isEmpty()){if(out.length()>0)out.append('\n');out.append(value);}}
}
