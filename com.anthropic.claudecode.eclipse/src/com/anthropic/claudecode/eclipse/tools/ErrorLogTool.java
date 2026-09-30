package com.anthropic.claudecode.eclipse.tools;

import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.List;
import java.util.Locale;

import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.Platform;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.tools.ErrorLogParser.Entry;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Reads Eclipse's own error log — the workspace {@code .metadata/.log} the Error Log view
 * shows. This is where exceptions thrown inside the IDE land: a plug-in failing to start, a
 * handler throwing, a builder crashing. None of it reaches a Console, so without this the
 * only way to see it is to open the Error Log view and read it out.
 *
 * <p>Reads the file rather than the view, so it works whether or not the view is open and
 * relies on nothing but {@link Platform#getLogFileLocation()} in
 * {@code org.eclipse.core.runtime}, a hard {@code Require-Bundle} — no availability guard.
 * Only the current {@code .log} is read, not the rolled-over {@code .bak_N.log} files. Any
 * other log can be named with {@code path}, which is how the log of an Eclipse Application
 * started from this IDE is read: it writes into its own runtime workspace.
 */
public class ErrorLogTool implements McpTool {

    private static final int DEFAULT_LIMIT = 20;
    private static final int MAX_LIMIT = 200;
    private static final int MAX_MESSAGE_CHARS = 2000;
    private static final int MAX_STACK_LINES = 40;

    @Override
    public String toolName() {
        return "errorLog";
    }

    @Override
    public String description() {
        return "Read Eclipse's error log (the Error Log view), newest entries first: exceptions and "
                + "warnings from the IDE and its plug-ins, with their stack traces. Use it when "
                + "something in Eclipse failed without a visible error. An Eclipse Application "
                + "started with 'runAs' logs into its own workspace: pass 'path' as that workspace's "
                + ".metadata/.log to read it. Defaults to warnings and errors from the current session.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");

        JsonObject props = new JsonObject();

        JsonObject severity = new JsonObject();
        severity.addProperty("type", "string");
        severity.addProperty("description",
                "Least severe entries to include: error, warning (default), or info (everything).");
        JsonArray severities = new JsonArray();
        severities.add("error");
        severities.add("warning");
        severities.add("info");
        severity.add("enum", severities);
        props.add("severity", severity);

        JsonObject plugin = new JsonObject();
        plugin.addProperty("type", "string");
        plugin.addProperty("description",
                "Only entries from plug-ins whose id contains this text, e.g. 'org.eclipse.jdt'.");
        props.add("plugin", plugin);

        JsonObject limit = new JsonObject();
        limit.addProperty("type", "integer");
        limit.addProperty("description",
                "How many entries to return, newest first (default " + DEFAULT_LIMIT
                        + ", at most " + MAX_LIMIT + ").");
        props.add("limit", limit);

        JsonObject allSessions = new JsonObject();
        allSessions.addProperty("type", "boolean");
        allSessions.addProperty("description",
                "Include entries from earlier IDE sessions still in the log (default false: only "
                        + "since this Eclipse started).");
        props.add("allSessions", allSessions);

        JsonObject path = new JsonObject();
        path.addProperty("type", "string");
        path.addProperty("description",
                "Another log file to read instead of this IDE's own, e.g. the .metadata/.log in "
                        + "the workspace of an Eclipse Application started with 'runAs'.");
        props.add("path", path);

        schema.add("properties", props);
        return schema;
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        try {
            String severityParam = str(params, "severity");
            int minRank = "error".equalsIgnoreCase(severityParam) ? 3
                    : "info".equalsIgnoreCase(severityParam) ? 1 : 2;
            String plugin = str(params, "plugin");
            String needle = plugin == null ? null : plugin.toLowerCase(Locale.ROOT);
            int limit = DEFAULT_LIMIT;
            if (params.has("limit") && !params.get("limit").isJsonNull()) {
                limit = Math.max(1, Math.min(MAX_LIMIT, params.get("limit").getAsInt()));
            }
            boolean allSessions = params.has("allSessions") && !params.get("allSessions").isJsonNull()
                    && params.get("allSessions").getAsBoolean();

            File file;
            String path = str(params, "path");
            if (path != null) {
                file = new File(path);
            } else {
                IPath location = Platform.getLogFileLocation();
                if (location == null) return McpToolResult.error("This Eclipse has no log file location.");
                file = location.toFile();
            }

            JsonObject out = new JsonObject();
            out.addProperty("path", file.getAbsolutePath());
            if (path != null && !file.isFile()) {
                return McpToolResult.error("No such log file: " + path);
            }
            if (!file.isFile()) {
                out.addProperty("matched", 0);
                out.add("entries", new JsonArray());
                out.addProperty("note", "The log file does not exist yet — nothing has been logged.");
                return McpToolResult.success(out);
            }

            ErrorLogParser.Result parsed = ErrorLogParser.parse(
                    new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8));
            List<Entry> entries = parsed.entries();

            JsonArray listed = new JsonArray();
            int matched = 0;
            for (int i = entries.size() - 1; i >= 0; i--) {
                Entry e = entries.get(i);
                if (!allSessions && e.session != parsed.sessions()) continue;
                if (ErrorLogParser.rank(e.worstSeverity()) < minRank) continue;
                if (needle != null && !e.mentionsPlugin(needle)) continue;
                matched++;
                if (listed.size() < limit) listed.add(toJson(e));
            }

            out.addProperty("matched", matched);
            if (matched > listed.size()) {
                out.addProperty("note", "Showing the " + listed.size() + " newest of " + matched
                        + " matching entries.");
            }
            out.add("entries", listed);
            ClaudeCodeView.debug("[errorLog] " + file + ": " + entries.size() + " entries, "
                    + matched + " matched, " + listed.size() + " returned");
            return McpToolResult.success(out);
        } catch (Exception e) {
            ClaudeCodeView.debug("[errorLog] failed: " + e);
            return McpToolResult.error("errorLog failed: " + e.getClass().getSimpleName()
                    + ": " + e.getMessage());
        }
    }

    private static JsonObject toJson(Entry e) {
        JsonObject j = new JsonObject();
        j.addProperty("severity", ErrorLogParser.severityName(e.severity));
        j.addProperty("plugin", e.plugin);
        if (!e.date.isEmpty()) j.addProperty("date", e.date);
        j.addProperty("message", clip(e.message.toString()));
        if (e.stack.length() > 0) j.addProperty("stack", clipLines(e.stack.toString()));
        if (!e.children.isEmpty()) {
            JsonArray children = new JsonArray();
            for (Entry c : e.children) children.add(toJson(c));
            j.add("children", children);
        }
        return j;
    }

    private static String clip(String s) {
        return s.length() <= MAX_MESSAGE_CHARS ? s : s.substring(0, MAX_MESSAGE_CHARS) + "…(truncated)";
    }

    private static String clipLines(String s) {
        String[] lines = s.split("\n", -1);
        if (lines.length <= MAX_STACK_LINES) return s;
        return String.join("\n", java.util.Arrays.copyOf(lines, MAX_STACK_LINES))
                + "\n…(" + (lines.length - MAX_STACK_LINES) + " more lines)";
    }

    private static String str(JsonObject params, String key) {
        if (!params.has(key) || params.get(key).isJsonNull()) return null;
        String v = params.get(key).getAsString().trim();
        return v.isEmpty() ? null : v;
    }
}
