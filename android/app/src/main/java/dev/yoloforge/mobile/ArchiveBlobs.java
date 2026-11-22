package dev.yoloforge.mobile;

import android.util.Base64;
import java.io.*;
import java.security.MessageDigest;
import org.json.*;

/** Bounded downloads from Factory only; original attachment URIs are never fetched. */
final class ArchiveBlobs {
    static void validate(JSONObject descriptor)throws Exception {
        JSONObject file=descriptor.getJSONObject("blob");
        java.util.UUID.fromString(descriptor.getString("command_id"));
        if(!file.getString("sha256").matches("[a-f0-9]{64}") || file.getLong("bytes")<=0 || file.getLong("bytes")>64L*1024*1024)
            throw new Exception("Ungueltige Archivdatei");
    }
    static File download(Api api,JSONObject descriptor,File cache)throws Exception {
        validate(descriptor);JSONObject file=descriptor.getJSONObject("blob");
        String path="/v1/commands/"+descriptor.getString("command_id")+"/files/"+file.getString("sha256")+"/";
        File temporary=File.createTempFile("archive-blob-",".tmp",cache);boolean verified=false;
        try {
            MessageDigest digest=MessageDigest.getInstance("SHA-256");long offset=0,size=file.getLong("bytes");
            try(var output=new FileOutputStream(temporary)) {
                while(offset<size) {
                    JSONObject chunk=api.request("GET",path+offset,null),metadata=chunk.getJSONObject("file");
                    if(chunk.getLong("offset")!=offset)throw new Exception("Archivoffset geaendert");
                    for(String key:new String[]{"sha256","bytes","uri","name","mime","message_id","source_revision","captured_unix_ms"})
                        if(!file.get(key).toString().equals(metadata.get(key).toString()))throw new Exception("Archividentitaet geaendert");
                    String data=chunk.getString("data");if(data.length()>1398104)throw new Exception("Archivblock zu gross");
                    byte[] bytes=Base64.decode(data,Base64.NO_WRAP);
                    if(bytes.length==0 || bytes.length>1024*1024 || offset+bytes.length>size || (offset+bytes.length<size && bytes.length!=1024*1024))throw new Exception("Ungueltiger Archivblock");
                    output.write(bytes);digest.update(bytes);offset+=bytes.length;
                }
                output.getFD().sync();
            }
            StringBuilder hash=new StringBuilder();for(byte b:digest.digest())hash.append(String.format(java.util.Locale.ROOT,"%02x",b&255));
            if(!hash.toString().equals(file.getString("sha256")))throw new Exception("Archivdatei beschaedigt (SHA256)");
            verified=true;return temporary;
        } finally {if(!verified)temporary.delete();}
    }
}
