package com.anthropic.claudecode.eclipse.ui;

import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;

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
 * the install (Homebrew, apt, ...), printing a command like {@code "brew upgrade
 * claude-code"} rather than updating directly — {@link #runCommandAsync} runs exactly
 * that follow-up command, still only from an explicit second click.
 *
 * <p>This mutates the user's system, so it is only ever invoked from an explicit
 * user action (the update button in the Claude Code view) — never automatically,
 * and never as a side effect of the version check.
 */
public final class CliUpdateService {

    private CliUpdateService() {}

    /** An update run is slow (download + install); don't hang forever either. */
    private static final int TIMEOUT_MINUTES = 10;

    /** Result of an update attempt: {@code {ok, output}}. */
    public static final class Result {
        public final boolean ok;
        /** Combined stdout+stderr, trimmed — shown back to the user verbatim. */
        public final String output;

        Result(boolean ok, String output) {
            this.ok = ok;
            this.output = output;
        }

        public String toJson() {
            JsonObject o = new JsonObject();
            o.addProperty("ok", ok);
            o.addProperty("output", output);
            return o.toString();
        }
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
        runAsync(argv, "claude-cli-update", cb);
    }

    /**
     * Runs the exact command {@code claude update} itself printed when it deferred to a
     * system package manager (Homebrew, apt, ...) instead of updating directly — e.g.
     * {@code "brew upgrade claude-code"}. Still only ever reached from that same explicit
     * user action (clicking through the update banner's follow-up prompt), never
     * automatically.
     *
     * <p>Runs as argv, not through a shell, so there is nothing for shell metacharacters
     * in the parsed text to do even in principle — {@code commandLine} is split on
     * whitespace and executed as a literal program + arguments, exactly like {@link
     * #updateAsync}. The first token is resolved on PATH the same way {@code claude}
     * itself is, since a package manager binary (e.g. Homebrew's {@code brew}) has the
     * identical "not on the JVM's inherited PATH" problem on a Finder-launched Eclipse.app.
     *
     * @param commandLine e.g. {@code "brew upgrade claude-code"}; a no-op failure if blank
     */
    public static void runCommandAsync(String commandLine, Consumer<Result> cb) {
        String[] parts = commandLine == null ? new String[0] : commandLine.trim().split("\\s+");
        if (parts.length == 0 || parts[0].isEmpty()) {
            Result r = new Result(false, "No command to run.");
            try { cb.accept(r); } catch (Throwable ignored) {}
            return;
        }
        parts[0] = CliVersionService.resolveOnPath(parts[0]);
        runAsync(parts, "claude-managed-update", cb);
    }

    /** Shared spawn/capture/timeout logic for {@link #updateAsync} and {@link #runCommandAsync}. */
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
