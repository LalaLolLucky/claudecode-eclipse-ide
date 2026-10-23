package com.anthropic.claudecode.eclipse.ui;

import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.function.Consumer;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Asks GitHub for the latest published release of this plugin and compares it with the one
 * installed. Done only when the user asks (the command menu's "Check for updates"), never in
 * the background. Everything is off the UI thread and any failure (offline, rate limit,
 * a reply that is not a release) is "unknown", so the caller can always say something.
 *
 * <p>This only finds out; installing is Eclipse's own, through its update wizard.
 */
public final class PluginUpdateService {
    private PluginUpdateService() {}

    /** The repository's latest release, which the update site is built from. */
    private static final String LATEST_RELEASE_URL =
            "https://api.github.com/repos/eilonwy06/claudecode-eclipse-ide/releases/latest";

    /** Installed and latest versions, and whether an update is available. */
    public static final class Info {
        /** e.g. "3.2.23". */
        public final String installed;
        /** The latest release's version, or "" when it could not be read. */
        public final String latest;
        /** True only when both are known AND installed &lt; latest. */
        public final boolean updateAvailable;
        /** The release's page, for its notes; "" when unknown. */
        public final String url;

        Info(String installed, String latest, boolean updateAvailable, String url) {
            this.installed = installed;
            this.latest = latest;
            this.updateAvailable = updateAvailable;
            this.url = url;
        }

        /** JSON for the webview: {@code {installed, latest, updateAvailable, url}}. */
        public JsonObject toJson() {
            JsonObject o = new JsonObject();
            o.addProperty("installed", installed);
            o.addProperty("latest", latest);
            o.addProperty("updateAvailable", updateAvailable);
            o.addProperty("url", url);
            return o;
        }
    }

    /** Checks on a background thread and hands the result to {@code cb} (never null, never throws). */
    public static void checkAsync(String installed, Consumer<Info> cb) {
        Thread t = new Thread(() -> {
            Info info;
            try {
                info = check(installed);
            } catch (Throwable ex) {
                info = new Info(installed, "", false, "");
            }
            try { cb.accept(info); } catch (Throwable ignored) {}
        }, "claude-plugin-update-check");
        t.setDaemon(true);
        t.start();
    }

    /** Blocks on the network. */
    static Info check(String installed) throws Exception {
        HttpURLConnection conn = (HttpURLConnection) URI.create(LATEST_RELEASE_URL).toURL().openConnection();
        conn.setRequestMethod("GET");
        conn.setConnectTimeout(5000);
        conn.setReadTimeout(5000);
        // GitHub refuses a request that says nothing of who is asking.
        conn.setRequestProperty("User-Agent", "claudecode-eclipse-ide");
        conn.setRequestProperty("Accept", "application/vnd.github+json");
        if (conn.getResponseCode() != 200) return new Info(installed, "", false, "");
        String body;
        try (InputStream in = conn.getInputStream()) {
            body = new String(in.readAllBytes(), StandardCharsets.UTF_8);
        }
        return fromRelease(installed, JsonParser.parseString(body).getAsJsonObject());
    }

    /** The comparison, from a release as GitHub describes it. */
    static Info fromRelease(String installed, JsonObject release) {
        String latest = versionOfTag(release.has("tag_name") && !release.get("tag_name").isJsonNull()
                ? release.get("tag_name").getAsString() : "");
        String url = release.has("html_url") && !release.get("html_url").isJsonNull()
                ? release.get("html_url").getAsString() : "";
        return new Info(installed, latest, CliVersionService.isOlder(installed, latest), url);
    }

    /** "v3.2.23" → "3.2.23"; "" for a tag that is not a version. */
    static String versionOfTag(String tag) {
        String v = tag == null ? "" : tag.trim();
        if (v.startsWith("v") || v.startsWith("V")) v = v.substring(1);
        return v.matches("\\d+(\\.\\d+){1,3}") ? v : "";
    }
}
