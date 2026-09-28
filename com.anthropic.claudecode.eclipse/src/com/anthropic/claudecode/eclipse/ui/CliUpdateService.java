package com.anthropic.claudecode.eclipse.ui;

import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import java.util.function.Supplier;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import com.anthropic.claudecode.eclipse.Activator;
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
 * <p>The one exception is FreeBSD's claude-freebsd install, which puts a wrapper
 * at {@code /usr/local/bin/claude} that sets {@code DISABLE_UPDATES=1}. There
 * {@code claude update} prints "Updates are disabled by your administrator" and
 * exits 0, so the only real updater is {@code sudo claude-freebsd --update}.
 *
 * <p>An exit status of 0 is not trusted on its own: the installed version is read
 * before and after, and a run that leaves it unchanged is reported as a failure
 * with whatever the updater printed.
 *
 * <p>This mutates the user's system, so it is only ever invoked from an explicit
 * user action (the update button in the Claude Code view) — never automatically,
 * and never as a side effect of the version check.
 */
public final class CliUpdateService {

    private CliUpdateService() {}

    /** An update run is slow (download + install); don't hang forever either. */
    private static final int TIMEOUT_MINUTES = 10;

    /** A terminal waits on the user typing a password, so it gets longer. */
    private static final int TERMINAL_TIMEOUT_MINUTES = 30;

    /** claude-freebsd's manager script and the marker its generated wrapper carries. */
    private static final String FREEBSD_MANAGER = "/usr/local/bin/claude-freebsd";
    private static final String FREEBSD_BIN_DIR = "/usr/local/libexec/claude-code";
    private static final String FREEBSD_WRAPPER_MARK = "Managed by claude-freebsd";

    /** Result of an update attempt: {@code {ok, output, managed?, version, hint}}. */
    public static final class Result {
        public final boolean ok;
        /** Combined stdout+stderr, trimmed — shown back to the user verbatim. */
        public final String output;
        /** The package manager's command as the CLI printed it, when {@code claude update}
         *  deferred to one {@link #runManagedAsync} can run; otherwise null. For display. */
        public final String managed;
        /** Installed version after the run, or "" when unknown. */
        public final String version;
        /** Command the user can run in a terminal instead. */
        public final String hint;

        Result(boolean ok, String output, String managed, String version, String hint) {
            this.ok = ok;
            this.output = output;
            this.managed = managed;
            this.version = version;
            this.hint = hint;
        }

        Result(boolean ok, String output, String version, String hint) {
            this(ok, output, null, version, hint);
        }

        Result(boolean ok, String output) {
            this(ok, output, "", "claude update");
        }

        public String toJson() {
            JsonObject o = new JsonObject();
            o.addProperty("ok", ok);
            o.addProperty("output", output);
            if (managed != null) o.addProperty("managed", managed);
            o.addProperty("version", version);
            o.addProperty("hint", hint);
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
     * Runs the update on a background thread and reports the outcome. On success
     * the cached version info is invalidated so the next check reflects the new
     * build.
     *
     * @param claudeCmd the configured CLI command (falls back to {@code claude})
     */
    public static void updateAsync(String claudeCmd, Consumer<Result> cb) {
        String raw = (claudeCmd == null || claudeCmd.isBlank())
                ? com.anthropic.claudecode.eclipse.Constants.DEFAULT_CLAUDE_CMD : claudeCmd;
        // Same PATHEXT caveat as the version check: CreateProcess won't find
        // `claude.cmd` from the bare name "claude".
        final String cmd = CliVersionService.resolveOnPath(raw);
        runAsync("claude-cli-update", () -> {
            String before = CliVersionService.installedVersion(cmd);
            Result r = isFreeBSDManaged(cmd) ? updateFreeBSD() : run(List.of(cmd, "update"), TIMEOUT_MINUTES);
            Managed m = managedCommand(r.output);
            pendingManaged = m;
            if (m != null) return new Result(r.ok, r.output, m.shown, "", r.hint);
            // A package-manager deferral is advice, not an update: the page reads its
            // "is managed by" line, which verify() would bury in a failure message.
            if (r.ok && !r.output.contains("is managed by")) r = verify(cmd, before, r);
            return r;
        }, cb);
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
        runAsync("claude-managed-update", () -> run(List.of(argv), TIMEOUT_MINUTES), cb);
    }

    /** Shared background-thread/callback logic for {@link #updateAsync} and {@link #runManagedAsync}. */
    private static void runAsync(String threadName, Supplier<Result> work, Consumer<Result> cb) {
        Thread t = new Thread(() -> {
            Result r;
            try {
                r = work.get();
            } catch (Throwable ex) {
                r = new Result(false, String.valueOf(ex.getMessage()));
            }
            if (r.ok) CliVersionService.invalidate();
            try { cb.accept(r); } catch (Throwable ignored) {}
        }, threadName);
        t.setDaemon(true);
        t.start();
    }

    /**
     * Fails a run that exited 0 but left the version where it was. Updaters do this
     * when they are disabled or only print instructions, and trusting the exit
     * status is what made the banner report "updated" when nothing had changed.
     */
    private static Result verify(String cmd, String before, Result r) {
        String after = CliVersionService.installedVersion(cmd);
        if (before.isEmpty() || after.isEmpty()) {
            return new Result(true, r.output, after, r.hint);
        }
        if (!CliVersionService.isOlder(before, after)) {
            String said = firstLine(r.output);
            String msg = "still on " + after + (said.isEmpty() ? "" : " (" + said + ")");
            return new Result(false, msg, after, r.hint);
        }
        return new Result(true, r.output, after, r.hint);
    }

    /**
     * True when {@code cmd} is claude-freebsd's wrapper, or a link to the binary it
     * manages (the wrapper itself creates {@code ~/.local/bin/claude} pointing there).
     */
    static boolean isFreeBSDManaged(String cmd) {
        if (!Activator.isFreeBSD() || cmd == null) return false;
        try {
            Path p = Paths.get(cmd);
            if (!p.isAbsolute() || !Files.isRegularFile(p)) return false;
            Path real = p.toRealPath();
            if (real.startsWith(FREEBSD_BIN_DIR)) return true;
            byte[] head;
            try (InputStream in = Files.newInputStream(real)) {
                head = in.readNBytes(512);
            }
            return new String(head, StandardCharsets.ISO_8859_1).contains(FREEBSD_WRAPPER_MARK);
        } catch (Throwable t) {
            return false;
        }
    }

    /**
     * {@code claude-freebsd --update} needs root. Passwordless sudo or doas runs it
     * here; otherwise it opens a terminal so sudo can ask for the password, and
     * waits for that window to close. {@link #verify} then decides the outcome.
     */
    private static Result updateFreeBSD() {
        String hint = "sudo " + FREEBSD_MANAGER + " --update";
        if (!Files.isExecutable(Paths.get(FREEBSD_MANAGER))) {
            return new Result(false, FREEBSD_MANAGER + " not found", "", hint);
        }
        if (succeeds(List.of("sudo", "-n", "true"))) {
            return withHint(run(List.of("sudo", "-n", FREEBSD_MANAGER, "--update"), TIMEOUT_MINUTES), hint);
        }
        if (succeeds(List.of("doas", "-n", "true"))) {
            return withHint(run(List.of("doas", "-n", FREEBSD_MANAGER, "--update"), TIMEOUT_MINUTES), hint);
        }
        String script = hint + "; s=$?; echo; "
                + "if [ $s -eq 0 ]; then echo 'Done. Press Enter to close this window.'; "
                + "else echo \"Update failed (exit $s). Press Enter to close this window.\"; fi; "
                + "read _";
        List<String> term = terminalCommand("Update Claude Code", script);
        if (term == null) {
            return new Result(false, "updating needs root and no terminal was found", "", hint);
        }
        Result r = run(term, TERMINAL_TIMEOUT_MINUTES);
        // The terminal's own exit status says nothing about sudo's; verify() does.
        return new Result(true, r.ok ? "" : r.output, "", hint);
    }

    /** A blocking terminal command that runs {@code script}, or null when none is installed. */
    private static List<String> terminalCommand(String title, String script) {
        // --disable-server / --nofork keep the process alive until the window closes;
        // without them the call returns at once and the version check runs too early.
        if (onPath("xfce4-terminal")) {
            return List.of("xfce4-terminal", "--disable-server", "-T", title, "-x", "sh", "-c", script);
        }
        if (onPath("konsole")) {
            return List.of("konsole", "--nofork", "-p", "tabtitle=" + title, "-e", "sh", "-c", script);
        }
        if (onPath("xterm")) {
            return List.of("xterm", "-T", title, "-e", "sh", "-c", script);
        }
        return null;
    }

    private static boolean onPath(String name) {
        return !CliVersionService.resolveOnPath(name).equals(name);
    }

    private static boolean succeeds(List<String> argv) {
        return run(argv, 1).ok;
    }

    private static Result withHint(Result r, String hint) {
        return new Result(r.ok, r.output, r.version, hint);
    }

    /** Runs {@code argv} to completion; ok means exit status 0. */
    private static Result run(List<String> argv, int timeoutMinutes) {
        try {
            ProcessBuilder pb = new ProcessBuilder(argv);
            // Reaches the registry / package manager, so it needs the user's proxy
            // vars and PATH, not the JVM's sparse ones.
            CliVersionService.applyShellEnv(pb);
            // claude-freebsd's wrapper prints mount warnings on stderr; they would
            // otherwise be the first line of the message shown in the banner.
            pb.environment().putIfAbsent("CLAUDE_FBSD_NO_MOUNT_WARN", "1");
            pb.redirectErrorStream(true);
            Process p = pb.start();
            p.getOutputStream().close();   // nothing reads a password from us
            String out;
            try (InputStream in = p.getInputStream()) {
                out = new String(in.readAllBytes(), StandardCharsets.UTF_8);
            }
            if (!p.waitFor(timeoutMinutes, TimeUnit.MINUTES)) {
                p.destroyForcibly();
                return new Result(false, "Update timed out after " + timeoutMinutes + " minutes.");
            }
            return new Result(p.exitValue() == 0, out.trim());
        } catch (Throwable ex) {
            return new Result(false, String.valueOf(ex.getMessage()));
        }
    }

    private static String firstLine(String s) {
        if (s == null) return "";
        for (String line : s.split("\\R")) {
            if (!line.isBlank()) return line.trim();
        }
        return "";
    }
}
