package com.anthropic.claudecode.eclipse.ui;

import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.Consumer;

import com.anthropic.claudecode.eclipse.NativeCore;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Fetches the account's actually-available models from the Anthropic API
 * ({@code GET /v1/models}) so the Claude GUI model chooser is dynamic rather than
 * a hardcoded list. Curated to the latest of each family (Fable / Opus / Sonnet /
 * Haiku) — the CLI aliases (e.g. {@code opus}) already resolve to the latest, so
 * those are used as the {@code --model} values.
 *
 * <p>The call is the Rust core's ({@link NativeCore#accountModels}): it reads the OAuth
 * token where the CLI keeps it — {@code ~/.claude/.credentials.json}, the Keychain on
 * macOS, under {@code CLAUDE_CONFIG_DIR} when that is set — has the CLI renew a token
 * past its life, and hands back each model's id and name, never the token. A native
 * library from before that call falls back to reading the credentials file here, as
 * this class did before. Everything runs off the UI thread and degrades to a sensible
 * built-in list on any failure (offline, signed out, etc.), so the chooser always works;
 * {@link Fetched#builtIn} says when it did, so the view can ask again, and Debug mode
 * logs why.
 */
public final class ModelCatalog {

    private ModelCatalog() {}

    private static final String MODELS_URL = "https://api.anthropic.com/v1/models?limit=100";

    /** Families we surface, in display order. */
    private static final String[] FAMILY_ORDER = { "fable", "opus", "sonnet", "haiku" };

    /** What one fetch came to: the chooser's list as JSON, whether that is the built-in
     *  list standing in for the account's, and whether asking again could get the
     *  account's ({@link #worthRetrying}). */
    public record Fetched(String json, boolean builtIn, boolean worthRetrying) {}

    /** The account's models as the API lists them, or {@code null} with the core's
     *  {@code state} ({@code signed-out}, {@code expired}, {@code error}) and why. */
    record Answer(JsonArray data, String state, String why) {}

    private static String descFor(String family) {
        switch (family) {
            case "fable":  return "Latest flagship model";
            case "opus":   return "Most capable — best for complex tasks";
            case "sonnet": return "Balanced capability and speed";
            case "haiku":  return "Fastest for quick answers";
            default:        return "";
        }
    }

    /** Runs the fetch+curation on a background thread, then delivers what it came to
     *  (never null, its list never null) to {@code cb}. */
    public static void fetchCuratedAsync(String claudeCmd, Consumer<Fetched> cb) {
        Thread t = new Thread(() -> {
            Fetched fetched;
            try { fetched = fetchAndCurate(claudeCmd); }
            catch (Throwable ex) { fetched = builtIn("error", "the fetch failed: " + ex); }
            try { cb.accept(fetched); } catch (Throwable ignored) {}
        }, "claude-models-fetch");
        t.setDaemon(true);
        t.start();
    }

    private static Fetched fetchAndCurate(String claudeCmd) throws Exception {
        Answer answer;
        try {
            answer = answerOf(NativeCore.accountModels(claudeCmd));
        } catch (LinkageError olderLibrary) {
            ClaudeCodeView.debug("[models] the native library has no accountModels; reading the credentials file here");
            answer = fetchFromJava();
        }
        if (answer.data() == null) return builtIn(answer.state(), answer.why());
        String json = curate(answer.data());
        if (json == null) return builtIn("empty", "the account's list named no model");
        ClaudeCodeView.debug("[models] the account's list: " + answer.data().size() + " models");
        return new Fetched(json, false, false);
    }

    private static Fetched builtIn(String state, String why) {
        ClaudeCodeView.debug("[models] the built-in list, because " + state + ": " + why);
        return new Fetched(fallbackJson(), true, worthRetrying(state));
    }

    /** Whether a fetch that ended in {@code state} could come out differently if asked
     *  again: only an {@code error} (the network, the API, a timeout). A login that is
     *  {@code signed-out} or {@code expired} does not come right by asking — the core has
     *  already had the CLI renew it once — and each ask would start the CLI again, and on
     *  macOS read the Keychain again, which can put a prompt in front of the user. */
    static boolean worthRetrying(String state) {
        return "error".equals(state);
    }

    /** Reads the core's answer ({@link NativeCore#accountModels}). */
    static Answer answerOf(String answer) {
        JsonObject o;
        try {
            o = JsonParser.parseString(answer == null ? "" : answer).getAsJsonObject();
        } catch (RuntimeException e) {
            return new Answer(null, "error", "the core's answer could not be read");
        }
        JsonElement ok = o.get("ok");
        if (ok != null && ok.isJsonPrimitive() && ok.getAsBoolean() && o.has("data") && o.get("data").isJsonArray())
            return new Answer(o.getAsJsonArray("data"), null, null);
        String state = str(o, "state");
        return new Answer(null, state != null ? state : "error", String.valueOf(str(o, "error")));
    }

    /** The account's models fetched here, for a native library from before
     *  {@link NativeCore#accountModels}: the credentials file only, so no Keychain. */
    private static Answer fetchFromJava() throws Exception {
        String token = readOAuthToken();
        if (token == null || token.isEmpty())
            return new Answer(null, "signed-out", "no token in ~/.claude/.credentials.json");

        HttpURLConnection conn = (HttpURLConnection) URI.create(MODELS_URL).toURL().openConnection();
        conn.setRequestMethod("GET");
        conn.setConnectTimeout(4000);
        conn.setReadTimeout(4000);
        conn.setRequestProperty("Authorization", "Bearer " + token);
        conn.setRequestProperty("anthropic-version", "2023-06-01");
        int code = conn.getResponseCode();
        if (code == 401 || code == 403) return new Answer(null, "expired", "HTTP " + code);
        if (code != 200) return new Answer(null, "error", "HTTP " + code);

        String body;
        try (InputStream in = conn.getInputStream()) {
            body = new String(in.readAllBytes(), StandardCharsets.UTF_8);
        }
        JsonArray data = JsonParser.parseString(body).getAsJsonObject().getAsJsonArray("data");
        return data == null ? new Answer(null, "error", "the answer was not a model list") : new Answer(data, null, null);
    }

    /** The chooser's list for the account's models, or {@code null} when they name none. */
    static String curate(JsonArray data) {
        // Pick the latest model per family by numeric version comparison.
        Map<String, Best> best = new LinkedHashMap<>();
        for (var el : data) {
            if (!el.isJsonObject()) continue;
            JsonObject m = el.getAsJsonObject();
            String id = str(m, "id");
            if (id == null) continue;
            String display = str(m, "display_name");
            String core = id.replaceFirst("^claude-", "").replaceFirst("-\\d{8}$", "");
            String[] parts = core.split("-");
            if (parts.length == 0) continue;
            String family = parts[0];
            int[] ver = new int[Math.max(0, parts.length - 1)];
            for (int i = 1; i < parts.length; i++) ver[i - 1] = parseIntSafe(parts[i]);
            Best cur = best.get(family);
            if (cur == null || cmpVer(ver, cur.ver) > 0) {
                best.put(family, new Best(id, display != null ? display : id, ver));
            }
        }

        // Display order: the known families first (in our preferred order), then
        // any OTHER families the API returns — so a brand-new family name that
        // Anthropic ships later still appears automatically (fully dynamic), not
        // just new versions of existing families.
        java.util.List<String> ordered = new java.util.ArrayList<>();
        for (String fam : FAMILY_ORDER) if (best.containsKey(fam)) ordered.add(fam);
        best.keySet().stream()
            .filter(f -> !contains(FAMILY_ORDER, f))
            .sorted()
            .forEach(ordered::add);

        JsonArray out = new JsonArray();
        out.add(model("", "Default (recommended)", "Latest model · efficient for routine tasks"));
        for (String fam : ordered) {
            Best b = best.get(fam);
            boolean known = contains(FAMILY_ORDER, fam);
            // Known family → the stable CLI alias (auto-resolves to the latest, so
            // it self-updates). New/unknown family → the full model id, which the
            // CLI always accepts as --model even without a documented alias.
            String modelId = known ? fam : b.id;
            out.add(model(modelId, shortLabel(b.display), known ? descFor(fam) : "", b.id));
            if (fam.equals("sonnet")) {
                out.add(model("sonnet[1m]", shortLabel(b.display) + " (1M context)",
                        "1M-token context · draws from usage credits", b.id));
            }
        }
        return ordered.isEmpty() ? null : out.toString();
    }

    /** "Claude Opus 4.8" → "Opus 4.8". */
    private static String shortLabel(String display) {
        if (display == null) return "";
        return display.replaceFirst("^Claude ", "");
    }

    private static JsonObject model(String id, String label, String desc) {
        return model(id, label, desc, "");
    }

    /** {@code fullId} is the concrete model id (e.g. claude-sonnet-5) behind an alias,
     *  used for the "Switched to …" divider. */
    private static JsonObject model(String id, String label, String desc, String fullId) {
        JsonObject o = new JsonObject();
        o.addProperty("id", id);
        o.addProperty("label", label);
        o.addProperty("desc", desc);
        o.addProperty("fullId", fullId == null ? "" : fullId);
        return o;
    }

    /** Built-in list used when the API is unreachable — Fable enabled (it is available). */
    static String fallbackJson() {
        JsonArray out = new JsonArray();
        out.add(model("", "Default (recommended)", "Latest model · efficient for routine tasks"));
        out.add(model("fable", "Fable", descFor("fable")));
        out.add(model("opus", "Opus", descFor("opus")));
        out.add(model("sonnet", "Sonnet", descFor("sonnet")));
        out.add(model("sonnet[1m]", "Sonnet (1M context)", "1M-token context · draws from usage credits"));
        out.add(model("haiku", "Haiku", descFor("haiku")));
        return out.toString();
    }

    private static String readOAuthToken() {
        try {
            Path p = Paths.get(System.getProperty("user.home"), ".claude", ".credentials.json");
            if (!Files.exists(p)) return null;
            JsonObject root = JsonParser.parseString(Files.readString(p)).getAsJsonObject();
            if (!root.has("claudeAiOauth") || !root.get("claudeAiOauth").isJsonObject()) return null;
            return str(root.getAsJsonObject("claudeAiOauth"), "accessToken");
        } catch (Throwable t) {
            return null;
        }
    }

    private static String str(JsonObject o, String k) {
        return (o.has(k) && o.get(k).isJsonPrimitive()) ? o.get(k).getAsString() : null;
    }

    private static int parseIntSafe(String s) {
        try { return Integer.parseInt(s); } catch (NumberFormatException e) { return 0; }
    }

    private static boolean contains(String[] arr, String v) {
        for (String s : arr) if (s.equals(v)) return true;
        return false;
    }

    private static int cmpVer(int[] a, int[] b) {
        int n = Math.max(a.length, b.length);
        for (int i = 0; i < n; i++) {
            int x = i < a.length ? a[i] : 0, y = i < b.length ? b[i] : 0;
            if (x != y) return Integer.compare(x, y);
        }
        return 0;
    }

    private static final class Best {
        final String id; final String display; final int[] ver;
        Best(String id, String display, int[] ver) { this.id = id; this.display = display; this.ver = ver; }
    }
}
