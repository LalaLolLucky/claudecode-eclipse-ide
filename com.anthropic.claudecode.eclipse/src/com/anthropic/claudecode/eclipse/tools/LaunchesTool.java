package com.anthropic.claudecode.eclipse.tools;

import org.eclipse.debug.core.DebugPlugin;
import org.eclipse.debug.core.ILaunch;
import org.eclipse.debug.core.ILaunchConfiguration;
import org.eclipse.debug.core.model.IProcess;
import org.eclipse.debug.core.model.IStreamMonitor;
import org.eclipse.debug.core.model.IStreamsProxy;
import org.eclipse.debug.ui.DebugUITools;
import org.eclipse.jface.text.IDocument;
import org.eclipse.ui.console.IConsole;
import org.eclipse.ui.console.TextConsole;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Lists the launches Eclipse is tracking, reads one's console output, and stops one — the
 * tool form of the Debug view's launch list, the Console view, and its red Terminate button.
 *
 * <p>It completes {@code runAs}, which returns as soon as a launch starts (or after
 * {@code waitSeconds}). For anything long-running — an Eclipse Application above all — the
 * output that arrives afterwards, and the means to stop it, live here. {@code runAs}
 * reports the launch configuration's name as {@code configuration}; that is what
 * {@code config} matches.
 *
 * <p>Output is read from the process's console document, not its stream monitors. Once
 * the Console attaches to a process it flushes the monitors into its document and turns
 * their buffering off ({@code ProcessConsole.StreamListener.flushAndDisableBuffer}), so
 * {@link IStreamMonitor#getContents()} is empty from then on. The monitors are read only
 * for a process with no console — "Allocate console" unchecked, or the console not created
 * yet — where they are still buffering.
 *
 * <p>Uses only {@code org.eclipse.debug.{core,ui}}, {@code org.eclipse.ui.console} and
 * {@code org.eclipse.jface.text}, all hard {@code Require-Bundle}s, so no availability
 * guard is needed.
 */
public class LaunchesTool implements McpTool {

    private static final int DEFAULT_MAX_CHARS = 8000;
    private static final int MAX_MAX_CHARS = 100_000;
    /** The Debug view keeps terminated launches until removed; list only the newest. */
    private static final int MAX_LISTED = 50;
    /** How long to wait for a stopped launch to report itself terminated. */
    private static final long STOP_WAIT_MS = 5000;

    @Override
    public String toolName() {
        return "launches";
    }

    @Override
    public String description() {
        return "List the launches Eclipse is tracking — anything started with Run As, Debug As, a "
                + "launch configuration, or the 'runAs' tool — read one's console output, or stop one. "
                + "action='list' (default) shows each launch's configuration name, mode, and whether "
                + "it has terminated. action='output' returns the tail of a launch's console: use it "
                + "to check on something 'runAs' started, such as an Eclipse Application, after "
                + "'runAs' has returned. action='stop' terminates a launch, like the Console's red "
                + "Terminate button, and requires 'config'. 'config' is the launch configuration "
                + "name, which 'runAs' reports as 'configuration'.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");

        JsonObject props = new JsonObject();

        JsonObject action = new JsonObject();
        action.addProperty("type", "string");
        action.addProperty("description", "list (default), output, or stop");
        JsonArray actions = new JsonArray();
        actions.add("list");
        actions.add("output");
        actions.add("stop");
        action.add("enum", actions);
        props.add("action", action);

        JsonObject config = new JsonObject();
        config.addProperty("type", "string");
        config.addProperty("description",
                "Launch configuration name, as shown by action='list' (case-insensitive). When "
                        + "several launches share it, the most recent running one is used. "
                        + "Optional for 'output', which then reads the most recent launch; "
                        + "required for 'stop'.");
        props.add("config", config);

        JsonObject maxChars = new JsonObject();
        maxChars.addProperty("type", "integer");
        maxChars.addProperty("description",
                "For 'output': how many characters from the end of each process's console to "
                        + "return (default " + DEFAULT_MAX_CHARS + ", at most " + MAX_MAX_CHARS + ").");
        props.add("maxChars", maxChars);

        schema.add("properties", props);
        return schema;   // no args means "list"
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        try {
            String action = str(params, "action");
            String config = str(params, "config");
            ClaudeCodeView.debug("[launches] action=" + action + " config=" + config);

            if (action == null || "list".equalsIgnoreCase(action)) {
                return McpToolResult.success(list());
            }
            if ("output".equalsIgnoreCase(action)) {
                int maxChars = DEFAULT_MAX_CHARS;
                if (params.has("maxChars") && !params.get("maxChars").isJsonNull()) {
                    maxChars = Math.max(1, Math.min(MAX_MAX_CHARS, params.get("maxChars").getAsInt()));
                }
                return output(config, maxChars);
            }
            if ("stop".equalsIgnoreCase(action)) {
                return stop(config);
            }
            return McpToolResult.error("Unknown action: " + action + ". Use list, output or stop.");
        } catch (Exception e) {
            ClaudeCodeView.debug("[launches] failed: " + e);
            return McpToolResult.error("launches failed: " + e.getClass().getSimpleName()
                    + ": " + e.getMessage());
        }
    }

    // ── Actions ─────────────────────────────────────────────────────────────────────

    private JsonObject list() {
        ILaunch[] launches = DebugPlugin.getDefault().getLaunchManager().getLaunches();
        JsonArray listed = new JsonArray();
        // getLaunches() is in the order the launches were added, so walk it backwards to
        // list the newest first.
        for (int i = launches.length - 1; i >= 0 && listed.size() < MAX_LISTED; i--) {
            JsonObject lj = header(launches[i]);
            JsonArray processes = new JsonArray();
            for (IProcess p : launches[i].getProcesses()) processes.add(processHeader(p));
            lj.add("processes", processes);
            listed.add(lj);
        }
        JsonObject out = new JsonObject();
        out.addProperty("count", launches.length);
        if (launches.length > listed.size()) {
            out.addProperty("note", "Showing the " + listed.size() + " most recent.");
        }
        out.add("launches", listed);
        return out;
    }

    private McpToolResult output(String config, int maxChars) {
        ILaunch launch = pick(config);
        if (launch == null) return noSuchLaunch(config);

        JsonObject out = header(launch);
        JsonArray processes = new JsonArray();
        for (IProcess p : launch.getProcesses()) {
            JsonObject pj = processHeader(p);
            ConsoleTail console = consoleTail(p, maxChars);
            if (console != null) {
                pj.addProperty("source", "console");
                pj.addProperty("totalChars", console.totalChars());
                if (!console.text().isEmpty()) pj.addProperty("output", console.text());
            } else {
                pj.addProperty("source", "streams");
                IStreamsProxy streams = p.getStreamsProxy();
                String stdout = streams == null ? "" : contents(streams.getOutputStreamMonitor());
                String stderr = streams == null ? "" : contents(streams.getErrorStreamMonitor());
                if (!stdout.isEmpty()) pj.addProperty("stdout", tail(stdout, maxChars));
                if (!stderr.isEmpty()) pj.addProperty("stderr", tail(stderr, maxChars));
                if (stdout.isEmpty() && stderr.isEmpty() && p.isTerminated()) {
                    // Without this, "no output" reads as "the program printed nothing".
                    pj.addProperty("note", "No console and no buffered output. Either the process "
                            + "printed nothing, or its console was closed or removed after the "
                            + "output had already been handed to it.");
                }
            }
            processes.add(pj);
        }
        out.add("processes", processes);
        if (processes.isEmpty()) {
            out.addProperty("note", "This launch has no processes, so there is no output to read.");
        }
        return McpToolResult.success(out);
    }

    private McpToolResult stop(String config) throws InterruptedException {
        if (config == null) {
            return McpToolResult.error("action='stop' requires 'config' — the launch configuration "
                    + "name from action='list'.");
        }
        ILaunch launch = pick(config);
        if (launch == null) return noSuchLaunch(config);

        JsonObject out = header(launch);
        if (launch.isTerminated()) {
            out.addProperty("stopped", false);
            out.addProperty("note", "Already terminated.");
            return McpToolResult.success(out);
        }
        if (!launch.canTerminate()) {
            return McpToolResult.error("Launch '" + nameOf(launch) + "' cannot be terminated.");
        }

        String failure = null;
        try {
            launch.terminate();
        } catch (Exception e) {
            // Still report where things ended up: a terminate that throws has often
            // stopped some of the launch's processes.
            failure = e.getClass().getSimpleName() + ": " + e.getMessage();
            ClaudeCodeView.debug("[launches] terminate failed: " + e);
        }
        long deadline = System.currentTimeMillis() + STOP_WAIT_MS;
        while (!launch.isTerminated() && System.currentTimeMillis() < deadline) {
            Thread.sleep(100);
        }

        out.addProperty("stopped", launch.isTerminated());
        out.addProperty("terminated", launch.isTerminated());
        if (failure != null) out.addProperty("error", failure);
        JsonArray processes = new JsonArray();
        for (IProcess p : launch.getProcesses()) processes.add(processHeader(p));
        out.add("processes", processes);

        int othersRunning = 0;
        for (ILaunch l : DebugPlugin.getDefault().getLaunchManager().getLaunches()) {
            if (l != launch && !l.isTerminated() && config.equalsIgnoreCase(nameOf(l))) othersRunning++;
        }
        if (othersRunning > 0) {
            out.addProperty("note", othersRunning + " other launch(es) named '" + config
                    + "' are still running; this stopped the most recent.");
        }
        ClaudeCodeView.debug("[launches] stop '" + nameOf(launch) + "' terminated="
                + launch.isTerminated() + (failure != null ? " error=" + failure : ""));
        return McpToolResult.success(out);
    }

    // ── Helpers ─────────────────────────────────────────────────────────────────────

    /**
     * The launch named {@code config}, or of all launches when it is null: the most recent
     * running one, else the most recent. A newer launch replaces an older one unless the
     * newer has terminated while the older is still running.
     */
    private static ILaunch pick(String config) {
        ILaunch best = null;
        for (ILaunch l : DebugPlugin.getDefault().getLaunchManager().getLaunches()) {
            if (config != null && !config.equalsIgnoreCase(nameOf(l))) continue;
            if (best == null || !l.isTerminated() || best.isTerminated()) best = l;
        }
        return best;
    }

    private static McpToolResult noSuchLaunch(String config) {
        if (config == null) return McpToolResult.error("Eclipse is not tracking any launches.");
        return McpToolResult.error("No launch named '" + config + "'. Call action='list' to see "
                + "the launches Eclipse is tracking.");
    }

    /** Launch configuration name, else the first process's label. Shared with {@link DebugTool}. */
    static String nameOf(ILaunch launch) {
        ILaunchConfiguration config = launch.getLaunchConfiguration();
        if (config != null) return config.getName();
        IProcess[] processes = launch.getProcesses();
        return processes.length > 0 ? processes[0].getLabel() : "(unnamed)";
    }

    private static JsonObject header(ILaunch launch) {
        JsonObject lj = new JsonObject();
        lj.addProperty("config", nameOf(launch));
        ILaunchConfiguration config = launch.getLaunchConfiguration();
        if (config != null) {
            try {
                lj.addProperty("type", config.getType().getName());
            } catch (Exception ignored) {
                // The configuration's type can be gone (its plug-in uninstalled); the rest stands.
            }
        }
        lj.addProperty("mode", launch.getLaunchMode());
        lj.addProperty("terminated", launch.isTerminated());
        return lj;
    }

    private static JsonObject processHeader(IProcess p) {
        JsonObject pj = new JsonObject();
        pj.addProperty("label", p.getLabel());
        pj.addProperty("terminated", p.isTerminated());
        if (p.isTerminated()) {
            try { pj.addProperty("exitValue", p.getExitValue()); } catch (Exception ignored) {}
        }
        return pj;
    }

    private record ConsoleTail(String text, int totalChars) {
    }

    /**
     * The last {@code maxChars} of the process's console, or null when it has no console.
     * Read on the UI thread, which is where the console appends to its document, and only
     * the tail — a console can hold a very large buffer.
     */
    private static ConsoleTail consoleTail(IProcess p, int maxChars) {
        return UiHelper.syncCall(() -> {
            IConsole console = DebugUITools.getConsole(p);
            if (!(console instanceof TextConsole textConsole)) return null;
            IDocument doc = textConsole.getDocument();
            int length = doc.getLength();
            int from = Math.max(0, length - maxChars);
            try {
                String text = doc.get(from, length - from);
                return new ConsoleTail(from > 0 ? "…(truncated)…\n" + text : text, length);
            } catch (Exception e) {
                return new ConsoleTail("", length);
            }
        });
    }

    private static String contents(IStreamMonitor monitor) {
        return monitor == null ? "" : monitor.getContents();
    }

    private static String tail(String s, int maxChars) {
        return s.length() <= maxChars ? s
                : "…(truncated)…\n" + s.substring(s.length() - maxChars);
    }

    private static String str(JsonObject params, String key) {
        if (!params.has(key) || params.get(key).isJsonNull()) return null;
        String v = params.get(key).getAsString().trim();
        return v.isEmpty() ? null : v;
    }
}
