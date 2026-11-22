package dev.yoloforge.mobile;

import android.util.Base64;
import java.security.MessageDigest;
import java.util.ArrayList;
import org.json.*;

/** Archive bytes are untrusted. Never fetch referenced URLs or use names as local paths. */
final class ArchiveContent {
    static final String KEY="__yoloforge_files_v1";
    static final int LIMIT=4*1024*1024;
    static byte[] bytes(JSONObject file)throws Exception {
        if(!"captured".equals(file.optString("status")))throw new Exception("Anhang nicht verfuegbar");
        String data=file.getString("data");if(data.length()>((LIMIT+2)/3)*4)throw new Exception("Anhang groesser als 4 MiB");
        byte[] bytes=Base64.decode(data,Base64.NO_WRAP);
        if(bytes.length>LIMIT || !digest(bytes).equals(file.getString("sha256")))throw new Exception("Anhang beschaedigt (SHA256)");
        return bytes;
    }
    private static String digest(byte[] bytes)throws Exception {
        byte[] digest=MessageDigest.getInstance("SHA-256").digest(bytes);StringBuilder out=new StringBuilder();
        for(byte b:digest)out.append(String.format(java.util.Locale.ROOT,"%02x",b&255));return out.toString();
    }
    static String safeName(String name){String safe=name.replaceAll("[^\\p{L}\\p{N}._-]","_");if(safe.isEmpty() || safe.equals(".") || safe.equals(".."))return "attachment.bin";return safe.substring(0,Math.min(160,safe.length()));}
    static ArrayList<JSONObject> files(JSONObject message)throws Exception {
        ArrayList<JSONObject> out=new ArrayList<>();JSONArray stored=message.optJSONArray(KEY);
        if(stored!=null){if(stored.length()>16)throw new Exception("Ungueltiger Anhangindex");for(int i=0;i<stored.length();i++)out.add(stored.getJSONObject(i));}
        inline(message,out,0);return out;
    }
    private static void inline(Object value,ArrayList<JSONObject> out,int depth)throws Exception {
        if(depth>32 || out.size()>=32)return;
        if(value instanceof JSONObject){JSONObject object=(JSONObject)value;
            String mime=object.optString("mime"),uri=object.optString("uri");String data=null;
            if(!mime.isEmpty()){int split=uri.indexOf(";base64,");if(uri.startsWith("data:") && split>=0)data=uri.substring(split+8);else if(object.opt("data") instanceof String)data=object.getString("data");}
            if(data!=null){JSONObject file=new JSONObject().put("name",object.optString("name","attachment.bin")).put("mime",mime).put("status","invalid_inline_data_or_over_limit");
                if(data.length()<=((LIMIT+2)/3)*4)try{byte[] bytes=Base64.decode(data,Base64.NO_WRAP);if(bytes.length<=LIMIT)file.put("status","captured").put("data",data).put("sha256",digest(bytes));}catch(IllegalArgumentException ignored){}
                out.add(file);
            }
            var keys=object.keys();while(keys.hasNext()){String key=keys.next();if(!KEY.equals(key))inline(object.get(key),out,depth+1);}
        }else if(value instanceof JSONArray){JSONArray array=(JSONArray)value;for(int i=0;i<array.length();i++)inline(array.get(i),out,depth+1);}
    }
    static String render(JSONObject message)throws Exception{return ((JSONObject)display(message,0)).toString(2);}
    private static Object display(Object value,int depth)throws Exception {
        if(depth>32)return "[Verschachtelungsgrenze]";
        if(value instanceof JSONObject){JSONObject input=(JSONObject)value,out=new JSONObject();var keys=input.keys();while(keys.hasNext()){String key=keys.next();if(KEY.equals(key))continue;Object child=input.get(key);
            if((key.equals("data") && input.has("mime")) || (key.equals("uri") && child instanceof String && ((String)child).startsWith("data:")))out.put(key,"[Binaeranhang separat speichern]");else out.put(key,display(child,depth+1));}return out;
        }if(value instanceof JSONArray){JSONArray input=(JSONArray)value,out=new JSONArray();for(int i=0;i<input.length();i++)out.put(display(input.get(i),depth+1));return out;}return value;
    }
}
