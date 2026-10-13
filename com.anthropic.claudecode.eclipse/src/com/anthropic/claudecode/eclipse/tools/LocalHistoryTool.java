package com.anthropic.claudecode.eclipse.tools;

import java.io.InputStream;
import java.nio.charset.Charset;
import java.time.Instant;
import java.util.List;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IFileState;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.runtime.NullProgressMonitor;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * A file's local history — the earlier contents Eclipse keeps each time a file is changed
 * or deleted through the workspace (Team &gt; Show Local History): listing them, reading one,
 * and putting one back. It is Eclipse's own and has nothing to do with version control, so
 * it also holds what was never committed.
 *
 * <p>Uses only {@code org.eclipse.core.resources}, already a hard {@code Require-Bundle}.
 */
public class LocalHistoryTool implements McpTool {

    private static final List<String> ACTIONS = List.of("list", "read", "restore");

    /** Cap on what 'read' returns, so one large revision can't flood the response. */
    private static final int MAX_CHARS = 200_000;

    @Override
    public String toolName() {
        return "localHistory";
    }

    @Override
    public String description() {
        return "Read Eclipse's local history of a file: the earlier contents it kept whenever the "
                + "file was changed or deleted through the workspace, independent of version control. "
                + "action='list' (default) shows the revisions of 'file', newest first. action='read' "
                + "returns the contents of the one with 'revision'. action='restore' puts that "
                + "revision back as the file's contents — also for a file that has since been "
                + "deleted — and keeps what it replaces as a new revision, so it can be reversed. "
                + "Eclipse keeps revisions for a limited time and only for changes it saw: a file "
                + "changed outside Eclipse gets one when it is next refreshed.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject props = new JsonObject();
        props.add("action", ToolSupport.choice("What to do (default 'list').", ACTIONS));
        props.add("file", ToolSupport.prop("string", "Absolute path of the file, inside a workspace project."));
        props.add("revision", ToolSupport.prop("integer", "For 'read' / 'restore': the revision, as 'list' shows it."));
        return ToolSupport.schema(props, "file");
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        ClaudeCodeView.debug("[localHistory] " + params);
        try {
            String action = ToolSupport.str(params, "action");
            if (action == null) action = "list";
            if (!ACTIONS.contains(action)) {
                return McpToolResult.error("Unknown action: " + action + ". Use one of: "
                        + String.join(", ", ACTIONS) + ".");
            }
            String path = ToolSupport.str(params, "file");
            if (path == null) return McpToolResult.error("Missing required parameter: file");
            IFile file = ToolSupport.workspaceFile(path);
            if (file == null) return McpToolResult.error("Not a file in a workspace project: " + path);

            IFileState[] states = file.getHistory(new NullProgressMonitor());   // newest first
            if (action.equals("list")) return list(path, file, states);

            if (!params.has("revision")) return McpToolResult.error("'revision' is required. Use action 'list'.");
            long revision = params.get("revision").getAsLong();
            IFileState state = null;
            for (IFileState s : states) {
                if (s.getModificationTime() == revision) {
                    state = s;
                    break;
                }
            }
            if (state == null || !state.exists()) {
                return McpToolResult.error("The file has no revision " + revision + ". Use action 'list'.");
            }
            return action.equals("read") ? read(path, file, state) : restore(path, file, state);
        } catch (Exception e) {
            ClaudeCodeView.debug("[localHistory] failed: " + e);
            return McpToolResult.error("localHistory failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }

    private McpToolResult list(String path, IFile file, IFileState[] states) {
        JsonArray revisions = new JsonArray();
        for (IFileState s : states) {
            JsonObject r = new JsonObject();
            r.addProperty("revision", s.getModificationTime());
            r.addProperty("saved", Instant.ofEpochMilli(s.getModificationTime()).toString());
            revisions.add(r);
        }
        JsonObject result = new JsonObject();
        result.addProperty("file", path);
        result.addProperty("exists", file.exists());
        result.add("revisions", revisions);
        return McpToolResult.success(result);
    }

    private McpToolResult read(String path, IFile file, IFileState state) throws Exception {
        String charset = state.getCharset();
        if (charset == null) charset = file.getParent().getDefaultCharset();
        String text;
        try (InputStream in = state.getContents()) {
            text = new String(in.readAllBytes(), Charset.forName(charset));
        }
        JsonObject result = new JsonObject();
        result.addProperty("file", path);
        result.addProperty("revision", state.getModificationTime());
        if (text.length() > MAX_CHARS) {
            result.addProperty("truncated", true);
            result.addProperty("note", "Only the first " + MAX_CHARS + " of " + text.length() + " characters are shown.");
            text = text.substring(0, MAX_CHARS);
        }
        result.addProperty("content", text);
        return McpToolResult.success(result);
    }

    private McpToolResult restore(String path, IFile file, IFileState state) throws Exception {
        boolean existed = file.exists();
        if (existed) {
            if (ToolSupport.hasUnsavedChanges(file)) {
                return McpToolResult.error("The file has unsaved changes in its editor. Save or revert them first.");
            }
            file.setContents(state, IResource.FORCE | IResource.KEEP_HISTORY, new NullProgressMonitor());
        } else {
            if (!file.getParent().exists()) {
                return McpToolResult.error("The file's folder no longer exists: " + file.getParent().getLocation());
            }
            try (InputStream in = state.getContents()) {
                file.create(in, IResource.FORCE, new NullProgressMonitor());
            }
        }
        JsonObject result = new JsonObject();
        result.addProperty("file", path);
        result.addProperty("restored", state.getModificationTime());
        result.addProperty("recreated", !existed);
        return McpToolResult.success(result);
    }
}
