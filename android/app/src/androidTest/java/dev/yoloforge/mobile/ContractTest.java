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
}
