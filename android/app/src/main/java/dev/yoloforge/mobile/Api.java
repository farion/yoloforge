package dev.yoloforge.mobile;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import org.json.JSONObject;
import org.json.JSONArray;
import org.json.JSONTokener;

final class Api {
    static final String CLIENT_ID=java.util.UUID.randomUUID().toString();
    final String base;
    private final String token;
    Api(String base, String token) throws Exception { this.base = normalize(base); this.token = token; }
    static String normalize(String value) throws Exception {
        URI uri = new URI(value.trim()); String host = uri.getHost();
        boolean local = "127.0.0.1".equals(host) || "localhost".equals(host) || "10.0.2.2".equals(host);
        if (host == null || uri.getUserInfo() != null || uri.getQuery() != null || uri.getFragment() != null
            || !("https".equals(uri.getScheme()) || local && "http".equals(uri.getScheme())))
            throw new Exception("HTTPS-Adresse ohne Zugangsdaten eingeben (HTTP nur fuer lokalen Test)");
        return value.trim().replaceAll("/+$", "");
    }
    HttpURLConnection open(String path) throws Exception {
        HttpURLConnection connection = (HttpURLConnection) new URI(base + path).toURL().openConnection();
        connection.setInstanceFollowRedirects(false); connection.setConnectTimeout(10000); connection.setReadTimeout(20000);
        connection.setRequestProperty("x-factory-protocol", "1");
        connection.setRequestProperty("x-factory-client-id",CLIENT_ID);
        connection.setRequestProperty("x-factory-client-kind","android");
        connection.setRequestProperty("x-factory-client-version",BuildConfig.VERSION_NAME);
        if (token != null) connection.setRequestProperty("Authorization", "Bearer " + token);
        return connection;
    }
    JSONObject request(String method, String path, JSONObject body) throws Exception {
        return (JSONObject) requestJson(method,path,body);
    }
    JSONArray array(String path) throws Exception {
        return (JSONArray) requestJson("GET",path,null);
    }
    private Object requestJson(String method,String path,JSONObject body) throws Exception {
        HttpURLConnection connection = open(path);
        try {
            connection.setRequestMethod(method);
            if (body != null) {
                byte[] bytes = body.toString().getBytes(StandardCharsets.UTF_8);
                connection.setDoOutput(true); connection.setRequestProperty("Content-Type", "application/json"); connection.setFixedLengthStreamingMode(bytes.length);
                try (var stream = connection.getOutputStream()) { stream.write(bytes); }
            }
            int status = connection.getResponseCode();
            if (status < 200 || status >= 300) throw new HttpFailure(status);
            try (InputStream in = connection.getInputStream()) { return new JSONTokener(new String(read(in, 16 * 1024 * 1024), StandardCharsets.UTF_8)).nextValue(); }
        } finally { connection.disconnect(); }
    }
    static byte[] read(InputStream input, int maximum) throws Exception {
        ByteArrayOutputStream out = new ByteArrayOutputStream(); byte[] buffer = new byte[8192]; int count;
        while ((count = input.read(buffer)) != -1) {
            if (out.size() + count > maximum) throw new Exception("Groessenlimit ueberschritten");
            out.write(buffer, 0, count);
        }
        return out.toByteArray();
    }
    static final class HttpFailure extends Exception {
        final int status;
        HttpFailure(int status) { super("Factory HTTP " + status + (status == 401 ? ": Anmeldung widerrufen/ungueltig" : "")); this.status = status; }
    }
}
