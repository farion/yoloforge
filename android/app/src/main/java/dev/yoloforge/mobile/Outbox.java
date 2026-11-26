package dev.yoloforge.mobile;

import java.util.Arrays;
import org.json.JSONObject;

/** One encrypted immutable request; a late completion cannot erase a newer request. */
final class Outbox {
    private final Vault vault;
    Outbox(Vault vault){this.vault=vault;}
    void prepare(Api api,String path,JSONObject body)throws Exception{
        prepare(api,"POST",path,body);
    }
    void prepare(Api api,String method,String path,JSONObject body)throws Exception{
        synchronized(Vault.class){
            if(vault.get("pending")!=null)throw new Exception("Ein gespeicherter Auftrag muss zuerst zugestellt werden");
            body.getString("request_id");
            vault.put("pending",new JSONObject().put("base",api.base).put("method",method).put("path",path).put("body",body));
        }
    }
    JSONObject send(Api api)throws Exception{
        JSONObject pending;
        synchronized(Vault.class){pending=vault.get("pending");}
        if(pending==null)return null;
        if(!api.base.equals(pending.getString("base")))throw new Exception("Auftrag gehoert zu einer anderen Factory");
        try{
            JSONObject result=api.request(pending.optString("method","POST"),pending.getString("path"),pending.getJSONObject("body"));
            clear(pending,false);return result;
        }
        catch(Api.HttpFailure error){if(Arrays.asList(400,403,404,409,422).contains(error.status)){
            clear(pending,true);
        }throw error;}
    }
    private void clear(JSONObject sent,boolean restoreDraft)throws Exception{
        synchronized(Vault.class){
            JSONObject current=vault.get("pending");
            if(current!=null && current.getString("base").equals(sent.getString("base")) && current.getJSONObject("body").getString("request_id").equals(sent.getJSONObject("body").getString("request_id"))){
                if(restoreDraft && "/v1/tasks".equals(sent.getString("path")))vault.put("draft:"+sent.getString("base")+":"+sent.getJSONObject("body").getString("project_id"),sent.getJSONObject("body"));
                vault.put("pending",null);
            }
        }
    }
}
