package dev.yoloforge.mobile;

import android.app.Instrumentation;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;
import java.io.*;
import java.net.ServerSocket;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.*;
import org.json.JSONObject;

@RunWith(AndroidJUnit4.class)
public final class ContractTest {
    @Test public void updateTransportAndChecksumRejectForeignAssets()throws Exception {
        String hash=Updates.digest(new byte[]{0,1,2});assertEquals(hash,Updates.checksum(hash+"  yoloforge.apk\n","yoloforge.apk"));
        try{Updates.checksum(hash+"  other.apk","yoloforge.apk");fail("Foreign checksum filename");}catch(Exception expected){}
        for(String url:List.of("http://github.com/file","https://user:token@github.com/file","https://evil.example/file","file:///data/private")){
            try{Updates.download(url,1024);fail("Untrusted update transport");}catch(Exception expected){}
        }
        var context=getInstrumentation().getTargetContext();
        try{Updates.verify(context,new File(context.getApplicationInfo().sourceDir));fail("Installed version accepted as upgrade");}catch(Exception expected){}
    }
    @Test public void conditionalMultiChoiceFormPreservesTypesAndDrafts()throws Exception {
        Throwable[] failure={null};getInstrumentation().runOnMainSync(()->{try{
            var context=getInstrumentation().getTargetContext();var page=new android.widget.LinearLayout(context);
            var schema=new org.json.JSONArray("[{\"key\":\"count\",\"type\":\"integer\",\"default\":3},{\"key\":\"features\",\"type\":\"multiselect\",\"minItems\":1,\"options\":[{\"value\":\"mouse\",\"label\":\"Maussteuerung\"}],\"default\":[\"mouse\"]},{\"key\":\"hidden\",\"type\":\"string\",\"required\":true,\"when\":[{\"key\":\"count\",\"op\":\"eq\",\"value\":9}]},{\"key\":\"visible\",\"type\":\"string\",\"default\":\"retained\",\"when\":[{\"key\":\"count\",\"op\":\"eq\",\"value\":3}]}]");
            FormEditor editor=new FormEditor(context,page,schema,null);JSONObject answer=editor.answer();
            assertTrue("Integer must not become a JSON double",answer.get("count") instanceof Long);assertEquals("mouse",answer.getJSONArray("features").getString(0));assertFalse(answer.has("hidden"));assertEquals("retained",answer.getString("visible"));
            JSONObject draft=editor.draft();var restored=new FormEditor(context,new android.widget.LinearLayout(context),schema,draft);assertEquals(answer.toString(),restored.answer().toString());
            draft.put("count","9");var changed=new FormEditor(context,new android.widget.LinearLayout(context),schema,draft);try{changed.answer();fail("Conditional required field ignored");}catch(Exception expected){}
        }catch(Throwable e){failure[0]=e;}});if(failure[0]!=null)throw new AssertionError(failure[0]);
    }
    @Test public void archiveBinaryIntegrityAndInlineExtraction()throws Exception {
        JSONObject inline=new JSONObject("{\"mime\":\"application/octet-stream\",\"name\":\"../../proof.bin\",\"uri\":\"data:application/octet-stream;base64,AP9QREY=\"}");
        JSONObject message=new JSONObject().put("id","msg_test").put("text","Ungekuerzter Text").put("file",inline);
        var files=ArchiveContent.files(message);assertEquals(1,files.size());
        assertArrayEquals(new byte[]{0,(byte)255,80,68,70},ArchiveContent.bytes(files.get(0)));
        assertTrue(ArchiveContent.render(message).contains("Ungekuerzter Text"));assertFalse(ArchiveContent.render(message).contains("AP9QREY="));
        assertFalse(ArchiveContent.safeName("../../proof.bin").contains("/"));
        files.get(0).put("sha256","0".repeat(64));
        try{ArchiveContent.bytes(files.get(0));fail("Corrupt attachment exported");}catch(Exception expected){}
        JSONObject nested=new JSONObject().put(ArchiveContent.KEY,new org.json.JSONArray().put(files.get(0)));
        assertEquals("Snapshot metadata must not be decoded twice",1,ArchiveContent.files(nested).size());
    }
    private Instrumentation getInstrumentation(){return InstrumentationRegistry.getInstrumentation();}
    @Test
    public void testVaultAndEndpointPolicy()throws Exception{
        var context=getInstrumentation().getTargetContext();Vault vault=new Vault(context);
        vault.put("contract-test",new JSONObject().put("secret","sensitive-test-value"));
        assertEquals("sensitive-test-value",new Vault(context).get("contract-test").getString("secret"));
        assertFalse(context.getSharedPreferences("vault",0).getString("contract-test","").contains("sensitive-test-value"));
        vault.put("contract-test",null);
        assertEquals("https://factory.example",Api.normalize("https://factory.example/"));
        for(String bad:List.of("http://factory.example","https://user:secret@factory.example","https://factory.example/?token=secret","file:///data/private")){
            try{new Api(bad,"token");fail("Accepted unsafe endpoint");}catch(Exception expected){}
        }
    }
    @Test public void testLostReplyRetainsExactRequestAndRestartReplaysIt()throws Exception{
        Vault vault=new Vault(getInstrumentation().getTargetContext());vault.put("pending",null);
        try(ServerSocket server=new ServerSocket(0)){
            List<String> bodies=Collections.synchronizedList(new ArrayList<>());
            var executor=Executors.newSingleThreadExecutor();
            Future<?> responder=executor.submit(()->{try{
                for(int i=0;i<2;i++)try(var socket=server.accept()){
                    socket.setSoTimeout(10000);BufferedReader input=new BufferedReader(new InputStreamReader(socket.getInputStream(),StandardCharsets.UTF_8));
                    String first=input.readLine();assertTrue(first.startsWith("POST /v1/tasks "));int length=0;String line;
                    while(!(line=input.readLine()).isEmpty())if(line.toLowerCase(Locale.ROOT).startsWith("content-length:"))length=Integer.parseInt(line.substring(15).trim());
                    char[] chars=new char[length];int offset=0;while(offset<length){int n=input.read(chars,offset,length-offset);if(n<0)throw new EOFException();offset+=n;}bodies.add(new String(chars));
                    if(i==1){socket.getOutputStream().write("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".getBytes(StandardCharsets.UTF_8));socket.getOutputStream().flush();}
                }
            }catch(Exception error){throw new RuntimeException(error);}});
            Api api=new Api("http://127.0.0.1:"+server.getLocalPort(),"fixture-token");
            JSONObject body=new JSONObject().put("request_id",UUID.randomUUID().toString()).put("prompt","Preserve exactly");new Outbox(vault).prepare(api,"/v1/tasks",body);
            try{new Outbox(vault).send(api);}catch(Exception expected){}
            if(vault.get("pending")!=null){assertEquals(body.toString(),new Vault(getInstrumentation().getTargetContext()).get("pending").getJSONObject("body").toString());new Outbox(new Vault(getInstrumentation().getTargetContext())).send(api);}
            responder.get(20,TimeUnit.SECONDS);executor.shutdownNow();assertNull(vault.get("pending"));assertEquals(2,bodies.size());assertEquals(bodies.get(0),bodies.get(1));
        }
    }
    @Test public void testOutboxCannotRetargetAnotherFactory()throws Exception{
        Vault vault=new Vault(getInstrumentation().getTargetContext());vault.put("pending",null);
        JSONObject body=new JSONObject().put("request_id",UUID.randomUUID().toString());
        new Outbox(vault).prepare(new Api("https://first.example","token"),"/v1/tasks",body);
        try{new Outbox(vault).send(new Api("https://second.example","token"));fail("Retargeted immutable request");}catch(Exception expected){}
        assertEquals("https://first.example",vault.get("pending").getString("base"));vault.put("pending",null);
    }
    @Test public void testRejectedPromptRestoresEncryptedDraftAndFixedSession()throws Exception{
        Vault vault=new Vault(getInstrumentation().getTargetContext());vault.put("pending",null);
        try(ServerSocket server=new ServerSocket(0)){
            var executor=Executors.newSingleThreadExecutor();
            Future<?> responder=executor.submit(()->{try(var socket=server.accept()){
                socket.setSoTimeout(10000);BufferedReader input=new BufferedReader(new InputStreamReader(socket.getInputStream(),StandardCharsets.UTF_8));
                String line;int length=0;while(!(line=input.readLine()).isEmpty())if(line.toLowerCase(Locale.ROOT).startsWith("content-length:"))length=Integer.parseInt(line.substring(15).trim());
                for(int i=0;i<length;i++)if(input.read()<0)throw new EOFException();
                socket.getOutputStream().write("HTTP/1.1 422 Unprocessable Content\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".getBytes(StandardCharsets.UTF_8));socket.getOutputStream().flush();
            }catch(Exception error){throw new RuntimeException(error);}});
            try{
                Api api=new Api("http://127.0.0.1:"+server.getLocalPort(),"fixture-token");
                JSONObject body=new JSONObject().put("request_id",UUID.randomUUID().toString()).put("project_id","project").put("prompt","Keep this draft").put("requested_session","fixed-session");
                new Outbox(vault).prepare(api,"/v1/tasks",body);
                try{new Outbox(vault).send(api);fail("Expected rejection");}catch(Api.HttpFailure expected){assertEquals(422,expected.status);}
                responder.get(20,TimeUnit.SECONDS);assertNull(vault.get("pending"));
                String key="draft:"+api.base+":project";JSONObject draft=new Vault(getInstrumentation().getTargetContext()).get(key);
                assertEquals("Keep this draft",draft.getString("prompt"));assertEquals("fixed-session",draft.getString("requested_session"));vault.put(key,null);
            }finally{executor.shutdownNow();}
        }
    }
}
