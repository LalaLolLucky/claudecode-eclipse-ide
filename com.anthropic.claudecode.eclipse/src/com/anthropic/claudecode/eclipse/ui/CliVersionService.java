package com.anthropic.claudecode.eclipse.ui;

import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Reconciles the INSTALLED Claude Code CLI with the latest published release.
 *
 * <p>This is the missing half of {@link ModelCatalog}: that class asks the
 * Anthropic API which models the <em>account</em> may use, which says nothing
 * about what the <em>installed binary</em> understands. A CLI that predates a
 * model release will reject it, and because model aliases ({@code opus},
 * {@code sonnet}) resolve inside the CLI, an old binary silently resolves them to
 * an older model instead of erroring. So rather than trying to enumerate what the
 * binary supports — the ids are compiled into a ~265 MB executable, and scraping
 * them yields false positives like {@code claude-opus-4-6-fast} and
 * {@code claude-fable-5.md} — we surface the version and let the user update.
 *
 * <p>Everything is off the UI thread and degrades to "unknown" on any failure
 * (offline, no npm, CLI missing), so the caller can always render something. The
 * result is cached for the session; {@link #checkAsync} only ever does real work
 * once per {@link #CACHE_MS} window.
 */
public final class CliVersionService {

    private CliVersionService() {}

    /** npm dist-tag metadata for the published CLI. */
    private static final String REGISTRY_URL =
            "https://registry.npmjs.org/-/package/@anthropic-ai/claude-code/dist-tags";

    /** Re-check at most once an hour per Eclipse session. */
    private static final long CACHE_MS = 60L * 60L * 1000L;

    private static volatile Info cached;
    private static volatile long cachedAt;

    /** Installed + latest versions, and whether an update is available. */
    public static final class Info {
        /** e.g. "2.1.220", or "" when the CLI couldn't be run. */
        public final String installed;
        /** Latest published version, or "" when the registry was unreachable. */
        public final String latest;
        /** True only when both are known AND installed &lt; latest. */
        public final boolean updateAvailable;

        Info(String installed, String latest, boolean updateAvailable) {
            this.installed = installed;
            this.latest = latest;
            this.updateAvailable = updateAvailable;
        }

        /** JSON for the webview: {@code {installed, latest, updateAvailable}}. */
        public String toJson() {
            JsonObject o = new JsonObject();
            o.addProperty("installed", installed);
            o.addProperty("latest", latest);
            o.addProperty("updateAvailable", updateAvailable);
            return o.toString();
        }
    }

    /**
     * Resolves the installed and latest versions on a background thread, then hands
     * the result to {@code cb} (never null, never throws). Cached for an hour.
     *
     * @param claudeCmd the configured CLI command (falls back to {@code claude})
     */
    public static void checkAsync(String claudeCmd, Consumer<Info> cb) {
        Info c = cached;
        if (c != null && System.currentTimeMillis() - cachedAt < CACHE_MS) {
            try { cb.accept(c); } catch (Throwable ignored) {}
            return;
        }
        Thread t = new Thread(() -> {
            Info info;
            try {
                String installed = installedVersion(claudeCmd);
                String latest = latestVersion();
                info = new Info(installed, latest, isOlder(installed, latest));
            } catch (Throwable ex) {
                info = new Info("", "", false);
            }
            // A CLI that could not be run is not cached: installing it has to show up
            // on the next check (reopening the view), not an hour later.
            if (!info.installed.isEmpty()) {
                cached = info;
                cachedAt = System.currentTimeMillis();
            }
            try { cb.accept(info); } catch (Throwable ignored) {}
        }, "claude-cli-version");
        t.setDaemon(true);
        t.start();
    }

    /** Drops the cache so the next {@link #checkAsync} re-resolves (post-update). */
    public static void invalidate() {
        cached = null;
        cachedAt = 0L;
    }

    /** {@code claude --version} → "2.1.220" (the command prints "2.1.220 (Claude Code)"). */
    static String installedVersion(String claudeCmd) {
        String cmd = (claudeCmd == null || claudeCmd.isBlank())
                ? com.anthropic.claudecode.eclipse.Constants.DEFAULT_CLAUDE_CMD : claudeCmd;
        return commandVersion(cmd);
    }

    /** {@code node --version} → "22.12.0", or "" when Node.js isn't installed. Blocks. */
    static String nodeVersion() {
        return commandVersion("node");
    }

    /** First x.y.z in the output of {@code cmd --version}, or "" when it can't be run. */
    private static String commandVersion(String cmd) {
        cmd = resolveOnPath(cmd);
        try {
            ProcessBuilder pb = new ProcessBuilder(cmd, "--version");
            applyShellEnv(pb);
            pb.redirectErrorStream(true);
            Process p = pb.start();
            String out;
            try (InputStream in = p.getInputStream()) {
                out = new String(in.readAllBytes(), StandardCharsets.UTF_8);
            }
            if (!p.waitFor(10, java.util.concurrent.TimeUnit.SECONDS)) {
                p.destroyForcibly();
                return "";
            }
            Matcher m = Pattern.compile("(\\d+\\.\\d+\\.\\d+)").matcher(out);
            return m.find() ? m.group(1) : "";
        } catch (Throwable t) {
            return "";   // not on PATH, not installed, etc.
        }
    }

    /**
     * Resolves a bare command name against PATH (+ PATHEXT on Windows).
     *
     * <p>Necessary because {@code ProcessBuilder("claude")} calls
     * {@code CreateProcess}, which — unlike a shell — does NOT consult PATHEXT, so
     * the npm shim {@code claude.cmd} is never found and the launch fails with
     * "error=2, The system cannot find the file specified". This mirrors the
     * resolution the Rust launcher already does for spawning the CLI.
     *
     * @return the resolved absolute path, or {@code cmd} unchanged when it is
     *         already a path or nothing matched (let the OS try anyway)
     */
    static String resolveOnPath(String cmd) {
        if (cmd == null || cmd.isEmpty()) return cmd;
        // Already a path — the OS can handle it.
        if (cmd.indexOf('/') >= 0 || cmd.indexOf('\\') >= 0) return cmd;
        boolean windows = System.getProperty("os.name", "").toLowerCase().startsWith("windows");
        // On Windows the PATHEXT variants must be tried BEFORE the bare name: npm
        // installs both `claude` (a POSIX shell script Windows cannot execute) and
        // `claude.cmd` (the real shim) into the same directory, so matching the
        // extension-less file first would resolve to something unrunnable.
        List<String> candidates = new ArrayList<>();
        if (windows) {
            String exts = System.getenv("PATHEXT");
            if (exts == null || exts.isBlank()) exts = ".COM;.EXE;.BAT;.CMD";
            for (String ext : exts.split(";")) {
                if (!ext.isBlank()) candidates.add(cmd + ext.trim().toLowerCase());
            }
        }
        candidates.add(cmd);

        String hit = searchPath(System.getenv("PATH"), candidates);
        if (hit != null) return hit;
        // The JVM's PATH is not the user's PATH. An Eclipse.app started from Finder
        // or the Dock inherits launchd's /usr/bin:/bin:/usr/sbin:/sbin, which holds
        // no npm/Homebrew/nvm prefix — so a `claude` in /usr/local/bin is invisible
        // here even though every shell the user opens finds it. That silence is not
        // harmless: locateBinary() then returns null, the binary scan yields "{}",
        // and BOTH the model list and --thinking-display support are reported as
        // unavailable — the latter leaving thinking blocks unexpandable, which is
        // what this cost on macOS. The Rust half already captures the login shell's
        // environment for exactly this reason; reusing that capture keeps the two
        // halves resolving the CLI the same way. Empty on Windows, where the full
        // user environment is inherited already, so this is a no-op there.
        hit = searchPath(capturedShellPath(), candidates);
        if (hit != null) return hit;
        return cmd;
    }

    /** First candidate in {@code pathEnv} that is a real file, or null. */
    private static String searchPath(String pathEnv, List<String> candidates) {
        if (pathEnv == null || pathEnv.isEmpty()) return null;
        for (String dir : pathEnv.split(java.io.File.pathSeparator)) {
            if (dir.isBlank()) continue;
            for (String cand : candidates) {
                try {
                    Path p = Paths.get(dir.trim(), cand);
                    if (Files.isRegularFile(p)) return p.toString();
                } catch (Throwable ignored) {}
            }
        }
        return null;
    }

    /**
     * Puts the captured login-shell environment onto {@code pb}.
     *
     * <p>Resolving the command to an absolute path is only half the job. These
     * processes inherit the JVM's environment, which on a Finder-launched
     * Eclipse.app is missing everything the user's shell rc sets — proxy vars
     * above all. {@code claude update} reaches the npm registry, so behind a
     * corporate proxy it fails with no HTTPS_PROXY even though the resolved path
     * is perfectly correct; and PATH matters again the moment the CLI shells out
     * to npm itself. The Rust-spawned chat has had this environment all along
     * (see {@code shell_env}); these two spawns are the ones that never got it.
     *
     * <p><b>Strictly additive</b>, so this cannot regress a platform that already
     * works. PATH is <em>merged</em> rather than replaced — captured entries first,
     * then any the JVM had that the capture lacks — so a directory that resolved
     * before still resolves, which a wholesale overwrite could not promise. Every
     * other key is filled only when the JVM does not already define it, so a proxy
     * set deliberately for Eclipse (eclipse.ini, a wrapper script) still beats the
     * one in the user's shell rc.
     *
     * <p>A no-op on Windows, where the capture is empty because the full user
     * environment is inherited from the registry already.
     */
    static void applyShellEnv(ProcessBuilder pb) {
        try {
            String[] pairs = com.anthropic.claudecode.eclipse.NativeCore.shellEnvInject();
            if (pairs == null || pairs.length == 0) return;
            java.util.Map<String, String> env = pb.environment();
            for (String kv : pairs) {
                if (kv == null) continue;
                int eq = kv.indexOf('=');
                if (eq <= 0) continue;
                String key = kv.substring(0, eq);
                String val = kv.substring(eq + 1);
                String existing = env.get(key);
                if ("PATH".equals(key)) {
                    env.put(key, mergePath(val, existing));
                } else if (existing == null || existing.isEmpty()) {
                    env.put(key, val);
                }
            }
        } catch (Throwable ignored) {
            // Native library not loaded, or an environment that forbids mutation —
            // spawn with what we have rather than not at all.
        }
    }

    /**
     * {@code captured} followed by every entry of {@code existing} it does not
     * already contain, order preserved and no duplicates.
     *
     * <p>Captured entries lead because finding the user's real toolchain is the
     * point; the tail is kept so nothing that used to resolve stops resolving.
     */
    private static String mergePath(String captured, String existing) {
        if (existing == null || existing.isEmpty()) return captured;
        if (captured == null || captured.isEmpty()) return existing;
        String sep = java.io.File.pathSeparator;
        java.util.LinkedHashSet<String> merged = new java.util.LinkedHashSet<>();
        for (String dir : captured.split(sep)) if (!dir.isBlank()) merged.add(dir);
        for (String dir : existing.split(sep)) if (!dir.isBlank()) merged.add(dir);
        return String.join(sep, merged);
    }

    /** Login-shell PATH once resolved; null while still unknown. */
    private static volatile String shellPathCache;

    /**
     * PATH as the user's login shell reports it, or {@code ""} when unavailable.
     *
     * <p>Cached once found, because the native side spawns a login shell to compute
     * it. A failure is deliberately NOT cached: this can be called before the native
     * library is loaded, and caching that would strand the fallback for the session.
     */
    private static String capturedShellPath() {
        String known = shellPathCache;
        if (known != null) return known;
        String found = "";
        try {
            String[] pairs = com.anthropic.claudecode.eclipse.NativeCore.shellEnvInject();
            if (pairs != null) {
                for (String kv : pairs) {
                    if (kv != null && kv.startsWith("PATH=")) {
                        found = kv.substring("PATH=".length());
                        break;
                    }
                }
            }
        } catch (Throwable ignored) {
            // Native library not loaded yet — retry on the next call.
        }
        if (found.isEmpty()) return "";
        shellPathCache = found;
        return found;
    }

    /**
     * Latest published version from the npm registry. Uses the {@code latest}
     * dist-tag, which is what {@code npm install} and the CLI's own updater track
     * (note {@code stable} can legitimately lag {@code latest}).
     */
    private static String latestVersion() {
        try {
            HttpURLConnection conn = (HttpURLConnection) URI.create(REGISTRY_URL).toURL().openConnection();
            conn.setRequestMethod("GET");
            conn.setConnectTimeout(4000);
            conn.setReadTimeout(4000);
            if (conn.getResponseCode() != 200) return "";
            String body;
            try (InputStream in = conn.getInputStream()) {
                body = new String(in.readAllBytes(), StandardCharsets.UTF_8);
            }
            JsonObject root = JsonParser.parseString(body).getAsJsonObject();
            return (root.has("latest") && root.get("latest").isJsonPrimitive())
                    ? root.get("latest").getAsString() : "";
        } catch (Throwable t) {
            return "";   // offline / registry blocked
        }
    }

    /** Numeric semver compare; false unless both parse and {@code a} is strictly older. */
    static boolean isOlder(String a, String b) {
        if (a == null || b == null || a.isEmpty() || b.isEmpty()) return false;
        String[] pa = a.split("\\."), pb = b.split("\\.");
        int n = Math.max(pa.length, pb.length);
        for (int i = 0; i < n; i++) {
            int x = i < pa.length ? parseIntSafe(pa[i]) : 0;
            int y = i < pb.length ? parseIntSafe(pb[i]) : 0;
            if (x != y) return x < y;
        }
        return false;
    }

    private static int parseIntSafe(String s) {
        try { return Integer.parseInt(s.trim()); } catch (NumberFormatException e) { return 0; }
    }
}
