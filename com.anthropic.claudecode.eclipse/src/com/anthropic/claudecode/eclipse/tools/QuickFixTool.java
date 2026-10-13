package com.anthropic.claudecode.eclipse.tools;

import java.util.ArrayList;
import java.util.List;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IMarker;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.ui.IEditorPart;
import org.eclipse.ui.IMarkerResolution;
import org.eclipse.ui.ide.IDE;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Eclipse's quick fixes for the problems of a file — what Ctrl+1 on a problem, or Quick Fix
 * in the Problems view, offers: listing them, and applying one.
 *
 * <p>The fixes come from whichever plug-in owns the problem, through the workbench's marker
 * help registry ({@code org.eclipse.ui.ide}, a hard {@code Require-Bundle}), so this works
 * for any language whose tooling contributes them and needs no import of any of it. A fix
 * is the contributing plug-in's code and expects the UI thread; both actions run there.
 */
public class QuickFixTool implements McpTool {

    private static final List<String> ACTIONS = List.of("list", "apply");

    /** Cap on the problems listed, so a badly broken file can't flood the response. */
    private static final int MAX_PROBLEMS = 50;

    @Override
    public String toolName() {
        return "quickFix";
    }

    @Override
    public String description() {
        return "List or apply Eclipse's quick fixes for a file's problems — what Ctrl+1 offers on an "
                + "error or warning, such as adding an import, a cast, a missing method or an "
                + "unhandled exception. action='list' (default) shows each problem of 'file' (or only "
                + "those on 'line') with its 'problem' id and the fixes on offer. action='apply' runs "
                + "the fix whose label is 'fix', exactly as listed, on the problem with that id, then "
                + "saves the file. Problems are as current as the last build, so build first, and "
                + "list again after a fix: the ids change when the file is rebuilt. Refused when the "
                + "file has unsaved changes in an open editor.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject props = new JsonObject();
        props.add("action", ToolSupport.choice("What to do (default 'list').", ACTIONS));
        props.add("file", ToolSupport.prop("string", "Absolute path of the file, inside a workspace project."));
        props.add("line", ToolSupport.prop("integer", "For 'list': show only the problems on this 1-based line."));
        props.add("problem", ToolSupport.prop("integer", "For 'apply': the problem's id, as 'list' shows it."));
        props.add("fix", ToolSupport.prop("string", "For 'apply': the fix's label, exactly as 'list' shows it."));
        return ToolSupport.schema(props, "file");
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        ClaudeCodeView.debug("[quickFix] " + params);
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
            if (file == null || !file.exists()) return McpToolResult.error("Not a file in a workspace project: " + path);

            return action.equals("list") ? list(params, path, file) : apply(params, path, file);
        } catch (Exception e) {
            ClaudeCodeView.debug("[quickFix] failed: " + e);
            return McpToolResult.error("quickFix failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }

    private McpToolResult list(JsonObject params, String path, IFile file) throws Exception {
        int line = ToolSupport.integer(params, "line", 0, 0, Integer.MAX_VALUE);
        List<IMarker> wanted = new ArrayList<>();
        for (IMarker m : file.findMarkers(IMarker.PROBLEM, true, IResource.DEPTH_ZERO)) {
            if (line == 0 || m.getAttribute(IMarker.LINE_NUMBER, 0) == line) wanted.add(m);
        }
        List<IMarker> shown = wanted.subList(0, Math.min(MAX_PROBLEMS, wanted.size()));

        JsonArray problems = UiHelper.syncCall(() -> {
            JsonArray out = new JsonArray();
            for (IMarker m : shown) {
                JsonObject p = new JsonObject();
                p.addProperty("problem", m.getId());
                p.addProperty("line", m.getAttribute(IMarker.LINE_NUMBER, 0));
                p.addProperty("severity", switch (m.getAttribute(IMarker.SEVERITY, IMarker.SEVERITY_INFO)) {
                    case IMarker.SEVERITY_ERROR -> "error";
                    case IMarker.SEVERITY_WARNING -> "warning";
                    default -> "information";
                });
                p.addProperty("message", m.getAttribute(IMarker.MESSAGE, ""));
                JsonArray fixes = new JsonArray();
                for (IMarkerResolution r : IDE.getMarkerHelpRegistry().getResolutions(m)) fixes.add(r.getLabel());
                p.add("fixes", fixes);
                out.add(p);
            }
            return out;
        });
        if (problems == null) return McpToolResult.error("Eclipse's window is not available.");

        JsonObject result = new JsonObject();
        result.addProperty("file", path);
        result.addProperty("problemCount", wanted.size());
        result.add("problems", problems);
        if (wanted.size() > shown.size()) {
            result.addProperty("truncated", true);
            result.addProperty("note", "Only the first " + MAX_PROBLEMS + " problems are listed. Pass 'line' for one.");
        }
        return McpToolResult.success(result);
    }

    private McpToolResult apply(JsonObject params, String path, IFile file) throws Exception {
        String label = ToolSupport.str(params, "fix");
        if (!params.has("problem") || label == null) {
            return McpToolResult.error("'problem' and 'fix' are required. Use action 'list'.");
        }
        IMarker marker = file.findMarker(params.get("problem").getAsLong());
        if (marker == null || !marker.exists()) {
            return McpToolResult.error("The file has no problem with that id any more. Use action 'list'.");
        }
        if (ToolSupport.hasUnsavedChanges(file)) {
            return McpToolResult.error("The file has unsaved changes in its editor. Save or revert them first.");
        }
        String message = marker.getAttribute(IMarker.MESSAGE, "");

        // Null from the UI call means the fix ran; otherwise it is why it did not.
        String[] saved = new String[1];
        String refusal = UiHelper.syncCall(() -> {
            IMarkerResolution chosen = null;
            List<String> offered = new ArrayList<>();
            for (IMarkerResolution r : IDE.getMarkerHelpRegistry().getResolutions(marker)) {
                offered.add(r.getLabel());
                if (chosen == null && label.equals(r.getLabel())) chosen = r;
            }
            if (chosen == null) {
                return offered.isEmpty() ? "Eclipse offers no fix for that problem."
                        : "That problem has no fix labelled '" + label + "'. On offer: " + String.join(" | ", offered);
            }
            chosen.run(marker);
            // A fix edits the file through its editor, opening one when need be, and leaves
            // the change unsaved there. Saved here so that a build sees it.
            IEditorPart editor = ToolSupport.editorOf(file);
            if (editor != null && editor.isDirty()) {
                editor.doSave(new NullProgressMonitor());
                saved[0] = "saved";
            }
            return "";
        });
        if (refusal == null) return McpToolResult.error("Eclipse's window is not available.");
        if (!refusal.isEmpty()) return McpToolResult.error(refusal);

        JsonObject result = new JsonObject();
        result.addProperty("file", path);
        result.addProperty("applied", label);
        result.addProperty("problem", message);
        result.addProperty("saved", saved[0] != null);
        return McpToolResult.success(result);
    }
}
