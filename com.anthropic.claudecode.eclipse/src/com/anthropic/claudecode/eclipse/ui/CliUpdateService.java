package com.anthropic.claudecode.eclipse.ui;

import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import com.google.gson.JsonObject;

/**
 * Runs the CLI's own updater ({@code claude update}) on the user's behalf.
 *
 * <p>Deliberately delegates to the CLI rather than picking a package manager: the
 * binary ships via npm, the native installer, and Homebrew, and {@code claude
 * update} already knows which one installed <em>it</em>. Running e.g.
 * {@code npm i -g @anthropic-ai/claude-code} against a native install would leave
 * two different binaries on PATH.
 *
 * <p>{@code claude update} itself defers to the system package manager when one owns
 * the install (Homebrew, winget, apk), printing a command like {@code "brew upgrade
 * claude-code"} rather than updating directly — {@link #runManagedAsync} runs that
 * manager's update, still only from an explicit second click.
 *
 * <p>What gets run is decided HERE, never by the page: the view's web content may ask
 * for "the follow-up" but cannot name a program. {@link #managedCommand} recognizes the
 * commands the CLI prints, whole and exactly, and maps each to a fixed argument list;
 * anything else it printed is shown to the user and not run.
 *
 * <p>This mutates the user's system, so it is only ever invoked from an explicit
 * user action (the update button in the Claude Code view) — never automatically,
 * and never as a side effect of the version check.
 */
public final class CliUpdateService {

    private CliUpdateService() {}

    /** An update run is slow (download + install); don't hang forever either. */
    private static final int TIMEOUT_MINUTES = 10;

    /** Result of an update attempt: {@code {ok, output, managed?}}. */
    public static final class Result {
        public final boolean ok;
        /** Combined stdout+stderr, trimmed — shown back to the user verbatim. */
        public final String output;
        /** The package manager's command as the CLI printed it, when {@code claude update}
         *  deferred to one {@link #runManagedAsync} can run; otherwise null. For display. */
        public final String managed;

        Result(boolean ok, String output) {
            this(ok, output, null);
        }

        Result(boolean ok, String output, String managed) {
            this.ok = ok;
            this.output = output;
            this.managed = managed;
        }

        public String toJson() {
            JsonObject o = new JsonObject();
            o.addProperty("ok", ok);
            o.addProperty("output", output);
            if (managed != null) o.addProperty("managed", managed);
            return o.toString();
        }
    }

    /** A package manager's update: how the CLI words it, and what is actually executed. */
    static final class Managed {
        final String shown;
        final String[] argv;

        Managed(String shown, String... argv) {
            this.shown = shown;
            this.argv = argv;
        }
    }

    /** The follow-up the last {@code claude update} called for; taken by {@link #runManagedAsync}. */
    private static volatile Managed pendingManaged;

    private static final Pattern ANSI = Pattern.compile("\u001B\\[[0-9;]*[A-Za-z]");
    private static final Pattern BREW = Pattern.compile("brew upgrade (claude-code(?:@[a-z0-9.-]+)?)");

    /**
     * The package-manager update {@code claude update} asked for in {@code output}, or null.
     *
     * <p>The CLI prints {@code "Claude is managed by X."}, then {@code "To update, run:"}
     * (or {@code "To update manually, run:"}) and the command on the next line. Only its
     * three known commands are accepted, each matched in full, and each runs with the
     * arguments the CLI itself uses when it performs that update — which for winget adds
     * the flags that stop it waiting on a prompt nobody can see.
     */
    static Managed managedCommand(String output) {
        if (output == null || !output.contains("is managed by")) return null;
        String[] lines = ANSI.matcher(output).replaceAll("").split("\\R");
        String cmd = null;
        for (int i = 0; i < lines.length && cmd == null; i++) {
            if (!lines[i].trim().matches("(?i)to update(?: manually)?, run:")) continue;
            for (int j = i + 1; j < lines.length && cmd == null; j++) {
                if (!lines[j].isBlank()) cmd = lines[j].trim();
            }
        }
        if (cmd == null) return null;
        Matcher brew = BREW.matcher(cmd);
        if (brew.matches()) return new Managed(cmd, "brew", "upgrade", "--cask", brew.group(1));
        if (cmd.equals("winget upgrade Anthropic.ClaudeCode")) {
            return new Managed(cmd, "winget", "upgrade", "--id", "Anthropic.ClaudeCode",
                    "--exact", "--silent", "--disable-interactivity");
        }
        if (cmd.equals("apk upgrade claude-code")) return new Managed(cmd, "apk", "upgrade", "claude-code");
        return null;
    }

    /**
     * Runs {@code claude update} on a background thread and reports the outcome.
     * On success the cached version info is invalidated so the next check reflects
     * the new build.
     *
     * @param claudeCmd the configured CLI command (falls back to {@code claude})
     */
    public static void updateAsync(String claudeCmd, Consumer<Result> cb) {
        String raw = (claudeCmd == null || claudeCmd.isBlank())
                ? com.anthropic.claudecode.eclipse.Constants.DEFAULT_CLAUDE_CMD : claudeCmd;
        // Same PATHEXT caveat as the version check: CreateProcess won't find
        // `claude.cmd` from the bare name "claude".
        String[] argv = { CliVersionService.resolveOnPath(raw), "update" };
        runAsync(argv, "claude-cli-update", r -> {
            Managed m = managedCommand(r.output);
            pendingManaged = m;
            cb.accept(m == null ? r : new Result(r.ok, r.output, m.shown));
        });
    }

    /**
     * Runs the package manager's update that the last {@link #updateAsync} reported in
     * {@link Result#managed}. Takes no command: it runs what {@link #managedCommand}
     * settled on then, once, and fails if there is nothing pending. Still only ever
     * reached from that same explicit user action (clicking through the update banner's
     * follow-up prompt), never automatically.
     *
     * <p>The program is resolved on PATH the same way {@code claude} itself is, since a
     * package manager binary (e.g. Homebrew's {@code brew}) has the identical "not on the
     * JVM's inherited PATH" problem on a Finder-launched Eclipse.app.
     */
    public static void runManagedAsync(Consumer<Result> cb) {
        Managed m = pendingManaged;
        pendingManaged = null;
        if (m == null) {
            Result r = new Result(false, "No command to run.");
            try { cb.accept(r); } catch (Throwable ignored) {}
            return;
        }
        String[] argv = m.argv.clone();
        argv[0] = CliVersionService.resolveOnPath(argv[0]);
        runAsync(argv, "claude-managed-update", cb);
    }

    /** Shared spawn/capture/timeout logic for {@link #updateAsync} and {@link #runManagedAsync}. */
    private static void runAsync(String[] argv, String threadName, Consumer<Result> cb) {
        Thread t = new Thread(() -> {
            Result r;
            try {
                ProcessBuilder pb = new ProcessBuilder(argv);
                // Reaches the registry / package manager, so it needs the user's proxy
                // vars and PATH, not the JVM's sparse ones.
                CliVersionService.applyShellEnv(pb);
                pb.redirectErrorStream(true);
                Process p = pb.start();
                String out;
                try (InputStream in = p.getInputStream()) {
                    out = new String(in.readAllBytes(), StandardCharsets.UTF_8);
                }
                boolean done = p.waitFor(TIMEOUT_MINUTES, TimeUnit.MINUTES);
                if (!done) {
                    p.destroyForcibly();
                    r = new Result(false, "Update timed out after " + TIMEOUT_MINUTES + " minutes.");
                } else {
                    r = new Result(p.exitValue() == 0, out.trim());
                }
            } catch (Throwable ex) {
                r = new Result(false, String.valueOf(ex.getMessage()));
            }
            if (r.ok) CliVersionService.invalidate();
            try { cb.accept(r); } catch (Throwable ignored) {}
        }, threadName);
        t.setDaemon(true);
        t.start();
    }
}
