package dev.yoloforge.mobile;

import android.content.Context;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.os.Build;
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;
import org.json.*;

/** Explicit update download. Public release transport never receives Factory credentials. */
final class Updates {
    static final String REPOSITORY="farion/yoloforge";
    static JSONObject latest()throws Exception {
        JSONObject release=new JSONObject(new String(download("https://api.github.com/repos/"+REPOSITORY+"/releases/latest",2*1024*1024),StandardCharsets.UTF_8));
        if(release.optBoolean("draft") || release.optBoolean("prerelease"))throw new Exception("Kein stabiles Release verfuegbar");
        String tag=release.getString("tag_name");if(!tag.matches("v[0-9]+\\.[0-9]+\\.[0-9]+"))throw new Exception("Ungueltige Release-Version");
        String name="yoloforge-android-"+tag+".apk";JSONArray assets=release.getJSONArray("assets");String apk=null,sha=null;
        for(int i=0;i<assets.length();i++){JSONObject asset=assets.getJSONObject(i);String filename=asset.getString("name");if(filename.equals(name)){if(apk!=null)throw new Exception("Mehrdeutiges APK");apk=asset.getString("browser_download_url");}if(filename.equals(name+".sha256")){if(sha!=null)throw new Exception("Mehrdeutige Pruefsumme");sha=asset.getString("browser_download_url");}}
        if(apk==null || sha==null)throw new Exception("Dieses Release enthaelt noch kein signiertes Android-APK");
        String prefix="https://github.com/"+REPOSITORY+"/releases/download/"+tag+"/";
        if(!apk.equals(prefix+name)||!sha.equals(prefix+name+".sha256"))throw new Exception("Unerwartete Release-Asset-Adresse");
        return new JSONObject().put("tag",tag).put("name",name).put("apk",apk).put("sha256",sha);
    }
    static byte[] download(String url,int maximum)throws Exception {
        for(int redirect=0;redirect<5;redirect++){
            URI uri=new URI(url);String host=uri.getHost();
            if(!"https".equals(uri.getScheme()) || uri.getUserInfo()!=null || uri.getFragment()!=null || !Arrays.asList("api.github.com","github.com","release-assets.githubusercontent.com","objects.githubusercontent.com").contains(host))throw new Exception("Unzulaessiges Update-Downloadziel");
            HttpURLConnection connection=(HttpURLConnection)uri.toURL().openConnection();connection.setInstanceFollowRedirects(false);connection.setConnectTimeout(10000);connection.setReadTimeout(30000);connection.setRequestProperty("User-Agent","YoloForge-Android");
            try{int status=connection.getResponseCode();if(status>=300 && status<400){String location=connection.getHeaderField("Location");if(location==null)throw new Exception("Ungueltige Downloadumleitung");url=uri.resolve(location).toString();continue;}if(status!=200)throw new Exception("Release-Download HTTP "+status);
                if(connection.getContentLengthLong()>maximum)throw new Exception("Update groesser als erlaubt");try(InputStream in=connection.getInputStream()){return Api.read(in,maximum);}
            }finally{connection.disconnect();}
        }
        throw new Exception("Zu viele Downloadumleitungen");
    }
    static String digest(byte[] bytes)throws Exception {StringBuilder hash=new StringBuilder();for(byte b:MessageDigest.getInstance("SHA-256").digest(bytes))hash.append(String.format(Locale.ROOT,"%02x",b&255));return hash.toString();}
    static String checksum(String text,String name)throws Exception {
        String[] parts=text.trim().split("\\s+");if(parts.length!=2 || !parts[0].matches("[0-9a-f]{64}") || !parts[1].equals(name))throw new Exception("Ungueltige APK-Pruefsumme");return parts[0];
    }
    static File fetch(Context context,JSONObject release)throws Exception {
        String hash=checksum(new String(download(release.getString("sha256"),4096),StandardCharsets.UTF_8),release.getString("name"));
        byte[] bytes=download(release.getString("apk"),64*1024*1024);if(!hash.equals(digest(bytes)))throw new Exception("APK-Pruefsumme stimmt nicht");
        File target=new File(context.getCacheDir(),"release-update.apk"),temp=File.createTempFile("update-",".apk",context.getCacheDir());
        try{try(FileOutputStream out=new FileOutputStream(temp)){out.write(bytes);out.getFD().sync();}verify(context,temp);if(!temp.renameTo(target))throw new Exception("APK konnte nicht gespeichert werden");return target;}finally{temp.delete();}
    }
    @SuppressWarnings("deprecation")
    static void verify(Context context,File file)throws Exception {
        PackageManager manager=context.getPackageManager();int flags=Build.VERSION.SDK_INT>=28?PackageManager.GET_SIGNING_CERTIFICATES:PackageManager.GET_SIGNATURES;
        PackageInfo archive=manager.getPackageArchiveInfo(file.getAbsolutePath(),flags),installed=manager.getPackageInfo(context.getPackageName(),flags);
        if(archive==null || !context.getPackageName().equals(archive.packageName))throw new Exception("APK ist nicht YoloForge");
        long version=Build.VERSION.SDK_INT>=28?archive.getLongVersionCode():archive.versionCode,current=Build.VERSION.SDK_INT>=28?installed.getLongVersionCode():installed.versionCode;
        if(version<=current)throw new Exception("Keine neuere Android-Version verfuegbar");
        Signature[] signatures=Build.VERSION.SDK_INT>=28?(archive.signingInfo==null?null:archive.signingInfo.getApkContentsSigners()):archive.signatures;
        Signature[] own=Build.VERSION.SDK_INT>=28?(installed.signingInfo==null?null:installed.signingInfo.getApkContentsSigners()):installed.signatures;
        if(signatures==null || signatures.length!=1 || !BuildConfig.RELEASE_CERTIFICATE.equals(digest(signatures[0].toByteArray())))throw new Exception("APK-Signatur stimmt nicht mit dem Release-Schluessel ueberein");
        if(own==null || own.length!=1 || !Arrays.equals(own[0].toByteArray(),signatures[0].toByteArray()))throw new Exception("Installierte App nutzt einen anderen Schluessel (z.B. Debug-Build)");
    }
}
