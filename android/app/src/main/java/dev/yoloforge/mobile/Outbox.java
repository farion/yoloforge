package dev.yoloforge.mobile;

import java.util.Arrays;
import org.json.JSONObject;

/** One encrypted immutable request; a late completion cannot erase a newer request. */
final class Outbox {
    private final Vault vault;
    Outbox(Vault vault){this.vault=vault;}
    void prepare(Api api,String path,JSONObject body)throws Exception{
        synchronized(Vault.class){
            if(vault.get("pending")!=null)throw new Exception("Ein gespeicherter Auftrag muss zuerst zugestellt werden");
            body.getString("request_id");
            vault.put("pending",new JSONObject().put("base",api.base).put("path",path).put("body",body));
        }
    }
    void send(Api api)throws Exception{
        JSONObject pending;
        synchronized(Vault.class){pending=vault.get("pending");}
        if(pending==null)return;
        if(!api.base.equals(pending.getString("base")))throw new Exception("Auftrag gehoert zu einer anderen Factory");
        try{api.request("POST",pending.getString("path"),pending.getJSONObject("body"));clear(pending);}
        catch(Api.HttpFailure error){if(Arrays.asList(400,403,404,409,422).contains(error.status))clear(pending);throw error;}
    }
    private void clear(JSONObject sent)throws Exception{
        synchronized(Vault.class){
            JSONObject current=vault.get("pending");
            if(current!=null && current.getString("base").equals(sent.getString("base")) && current.getJSONObject("body").getString("request_id").equals(sent.getJSONObject("body").getString("request_id")))vault.put("pending",null);
        }
    }
}
