package dev.yoloforge.mobile;

import android.content.Context;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.View;
import android.widget.*;
import java.util.*;
import org.json.*;

/** Native labeled form controls. Values retain their schema types on the wire. */
final class FormEditor {
    private final LinkedHashMap<String,Field> fields=new LinkedHashMap<>();
    private boolean building=true;
    private final class Field {
        final JSONObject schema;
        final LinearLayout group;
        View input;
        ArrayList<CheckBox> checks=new ArrayList<>();
        Field(JSONObject schema,LinearLayout group){this.schema=schema;this.group=group;}
        Object value(boolean typed)throws Exception {
            if(input instanceof Spinner){int position=((Spinner)input).getSelectedItemPosition();if(position<0)return JSONObject.NULL;return schema.getJSONArray("options").getJSONObject(position).get("value");}
            if(input instanceof CheckBox)return ((CheckBox)input).isChecked();
            if(schema.optString("type").equals("multiselect")){
                JSONArray values=new JSONArray(),options=schema.optJSONArray("options");HashSet<String> seen=new HashSet<>();
                for(int i=0;i<checks.size();i++)if(checks.get(i).isChecked()){String value=options.getJSONObject(i).getString("value");if(seen.add(value))values.put(value);}
                if(input instanceof EditText)for(String value:((EditText)input).getText().toString().split("\\r?\\n"))if(!value.isEmpty() && seen.add(value))values.put(value);
                return values;
            }
            String raw=((EditText)input).getText().toString();if(!typed || schema.optString("type").equals("string"))return raw;
            if(raw.isEmpty())return JSONObject.NULL;
            if(schema.optString("type").equals("integer"))return Long.valueOf(raw);
            double number=Double.parseDouble(raw);if(!Double.isFinite(number))throw new Exception("Endliche Zahl eingeben");return number;
        }
    }
    FormEditor(Context context,LinearLayout parent,JSONArray schemas,JSONObject draft)throws Exception {
        if(schemas.length()>64)throw new Exception("Zu viele Formularfelder");
        for(int i=0;i<schemas.length();i++){
            JSONObject schema=schemas.getJSONObject(i);String key=schema.getString("key"),type=schema.getString("type");
            if(fields.containsKey(key))throw new Exception("Doppelter Feldschluessel");
            if(!Arrays.asList("string","integer","number","boolean","multiselect").contains(type))throw new Exception("Feldtyp "+type+" bitte in OpenCode beantworten");
            JSONArray conditions=schema.optJSONArray("when");if(conditions!=null)for(int c=0;c<conditions.length();c++)if(!Arrays.asList("eq","neq").contains(conditions.getJSONObject(c).optString("op")))throw new Exception("Unbekannte Formularbedingung");
            LinearLayout group=new LinearLayout(context);group.setOrientation(LinearLayout.VERTICAL);parent.addView(group);
            Field field=new Field(schema,group);fields.put(key,field);
            String title=schema.optString("title",key);TextView heading=new TextView(context);heading.setText(title+(schema.optBoolean("required")?" *":""));group.addView(heading);
            String description=schema.optString("description");if(!description.isEmpty()){TextView note=new TextView(context);note.setText(description);group.addView(note);}
            Object initial=draft!=null && draft.has(key)?draft.opt(key):schema.opt("default");
            JSONArray options=schema.optJSONArray("options");
            if(type.equals("boolean")){CheckBox box=new CheckBox(context);box.setContentDescription(title);box.setChecked(Boolean.TRUE.equals(initial));box.setOnCheckedChangeListener((v,b)->visibility());field.input=box;group.addView(box);}
            else if(type.equals("multiselect")){
                HashSet<String> selected=new HashSet<>();if(initial instanceof JSONArray)for(int j=0;j<((JSONArray)initial).length();j++)selected.add(((JSONArray)initial).getString(j));
                if(options!=null)for(int j=0;j<options.length();j++){JSONObject option=options.getJSONObject(j);CheckBox box=new CheckBox(context);box.setText(label(option));box.setChecked(selected.remove(option.getString("value")));box.setOnCheckedChangeListener((v,b)->visibility());group.addView(box);field.checks.add(box);}
                if(schema.optBoolean("custom") || options==null || options.length()==0){EditText edit=edit(context,key,String.join("\n",selected),InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_FLAG_MULTI_LINE);edit.setHint("Weitere Werte, einer je Zeile");group.addView(edit);field.input=edit;}
            }else if(options!=null && options.length()>0 && !schema.optBoolean("custom")){
                Spinner spinner=new Spinner(context);ArrayList<String> labels=new ArrayList<>();int selected=0;
                for(int j=0;j<options.length();j++){JSONObject option=options.getJSONObject(j);labels.add(label(option));if(equal(initial,option.opt("value")))selected=j;}
                spinner.setContentDescription(title);spinner.setAdapter(new ArrayAdapter<>(context,android.R.layout.simple_spinner_dropdown_item,labels));spinner.setSelection(selected);
                spinner.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener(){public void onNothingSelected(AdapterView<?> p){visibility();}public void onItemSelected(AdapterView<?> p,View v,int n,long id){visibility();}});
                field.input=spinner;group.addView(spinner);
            }else{
                int inputType=type.equals("string")?InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_FLAG_MULTI_LINE:InputType.TYPE_CLASS_NUMBER|InputType.TYPE_NUMBER_FLAG_SIGNED|(type.equals("number")?InputType.TYPE_NUMBER_FLAG_DECIMAL:0);
                EditText edit=edit(context,key,initial==null || initial==JSONObject.NULL?"":initial.toString(),inputType);group.addView(edit);field.input=edit;
                if(options!=null)for(int j=0;j<options.length();j++){JSONObject option=options.getJSONObject(j);Button suggestion=new Button(context);suggestion.setText(label(option));String value=option.get("value").toString();suggestion.setOnClickListener(v->edit.setText(value));group.addView(suggestion);}
            }
        }
        building=false;visibility();
    }
    private EditText edit(Context context,String key,String initial,int type){EditText edit=new EditText(context);edit.setHint(key);edit.setContentDescription(key);edit.setInputType(type);edit.setText(initial);edit.addTextChangedListener(new TextWatcher(){public void beforeTextChanged(CharSequence s,int st,int count,int after){}public void onTextChanged(CharSequence s,int st,int before,int count){visibility();}public void afterTextChanged(Editable s){}});return edit;}
    private static String label(JSONObject option){String label=option.optString("label",option.optString("value")),description=option.optString("description");return description.isEmpty()?label:label+" — "+description;}
    private static boolean equal(Object a,Object b){if(a instanceof Number && b instanceof Number)return Double.compare(((Number)a).doubleValue(),((Number)b).doubleValue())==0;return Objects.equals(a,b);}
    private boolean enabled(Field field){JSONArray when=field.schema.optJSONArray("when");if(when==null)return true;for(int i=0;i<when.length();i++){JSONObject condition=when.optJSONObject(i);if(condition==null)return false;Field source=fields.get(condition.optString("key"));Object value=null;try{if(source!=null)value=source.value(true);}catch(Exception ignored){}boolean matches=equal(value,condition.opt("value"));if(condition.optString("op").equals("eq")?!matches:matches)return false;}return true;}
    private void visibility(){if(building)return;for(Field field:fields.values())field.group.setVisibility(enabled(field) && !field.schema.optBoolean("hidden")?View.VISIBLE:View.GONE);}
    JSONObject draft()throws Exception {JSONObject values=new JSONObject();for(var entry:fields.entrySet())values.put(entry.getKey(),entry.getValue().value(false));return values;}
    JSONObject answer()throws Exception {
        JSONObject values=new JSONObject();
        for(var entry:fields.entrySet()){Field field=entry.getValue();if(!enabled(field))continue;JSONObject schema=field.schema;Object value;
            try{value=field.value(true);}catch(Exception e){throw new Exception(schema.optString("title",entry.getKey())+": gueltigen Wert eingeben");}
            boolean empty=value==JSONObject.NULL || value instanceof String && ((String)value).isEmpty();if(empty){if(schema.optBoolean("required"))throw new Exception("Pflichtfeld fehlt: "+schema.optString("title",entry.getKey()));continue;}
            if(value instanceof String){String text=(String)value;int length=text.codePointCount(0,text.length());if(length<schema.optInt("minLength",0) || length>schema.optInt("maxLength",Integer.MAX_VALUE))throw new Exception("Textlaenge ungueltig: "+entry.getKey());}
            if(value instanceof Number){double number=((Number)value).doubleValue();if(number<schema.optDouble("minimum",-Double.MAX_VALUE)||number>schema.optDouble("maximum",Double.MAX_VALUE))throw new Exception("Zahl ausserhalb des Bereichs: "+entry.getKey());}
            if(value instanceof JSONArray){int count=((JSONArray)value).length();if(count<schema.optInt("minItems",schema.optBoolean("required")?1:0)||count>schema.optInt("maxItems",Integer.MAX_VALUE))throw new Exception("Anzahl der Auswahlwerte ungueltig: "+entry.getKey());}
            values.put(entry.getKey(),value);
        }
        return values;
    }
}
