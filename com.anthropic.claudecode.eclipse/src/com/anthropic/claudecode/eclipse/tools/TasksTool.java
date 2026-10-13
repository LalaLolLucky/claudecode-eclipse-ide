package com.anthropic.claudecode.eclipse.tools;

import org.eclipse.core.resources.IMarker;
import org.eclipse.core.resources.IProject;
import org.eclipse.core.resources.IResource;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Lists the task markers of the workspace — what the Tasks view shows: the TODO, FIXME and
 * XXX comments a builder found, and tasks added by hand. {@link GetDiagnosticsTool} reads
 * problem markers only; these are a different marker type.
 *
 * <p>Uses only {@code org.eclipse.core.resources}, already a hard {@code Require-Bundle}.
 */
public class TasksTool implements McpTool {

    private static final int LISTED = 200;
    private static final int MOST_LISTED = 2000;

    @Override
    public String toolName() {
        return "tasks";
    }

    @Override
    public String description() {
        return "List Eclipse's task markers (the Tasks view): the TODO, FIXME and XXX comments its "
                + "builders found in source files, and tasks added by hand, each with its file, line, "
                + "text and priority. Scope: the named projects, or the whole workspace when none are "
                + "given. Builders write these markers, so they are as current as the last build.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject props = new JsonObject();
        props.add("projects", ToolSupport.strings(
                "Project names to list tasks of. Omit or leave empty for the whole workspace."));
        props.add("limit", ToolSupport.prop("integer",
                "How many to list, 1 to " + MOST_LISTED + "; " + LISTED + " when left out."));
        return ToolSupport.schema(props);
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        ClaudeCodeView.debug("[tasks] " + params);
        try {
            int limit = ToolSupport.integer(params, "limit", LISTED, 1, MOST_LISTED);
            JsonArray tasks = new JsonArray();
            int total = 0;
            for (IProject project : ToolSupport.projects(params, "list tasks")) {
                for (IMarker marker : project.findMarkers(IMarker.TASK, true, IResource.DEPTH_INFINITE)) {
                    total++;
                    if (tasks.size() >= limit) continue;
                    JsonObject task = new JsonObject();
                    task.addProperty("project", project.getName());
                    IResource resource = marker.getResource();
                    if (resource.getLocation() != null) task.addProperty("file", resource.getLocation().toOSString());
                    task.addProperty("line", marker.getAttribute(IMarker.LINE_NUMBER, 0));
                    task.addProperty("message", marker.getAttribute(IMarker.MESSAGE, ""));
                    task.addProperty("priority", switch (marker.getAttribute(IMarker.PRIORITY, IMarker.PRIORITY_NORMAL)) {
                        case IMarker.PRIORITY_HIGH -> "high";
                        case IMarker.PRIORITY_LOW -> "low";
                        default -> "normal";
                    });
                    if (marker.getAttribute(IMarker.DONE, false)) task.addProperty("done", true);
                    tasks.add(task);
                }
            }
            JsonObject result = new JsonObject();
            result.addProperty("total", total);
            result.add("tasks", tasks);
            if (total > tasks.size()) result.addProperty("truncated", true);
            return McpToolResult.success(result);
        } catch (ToolSupport.Refusal e) {
            return McpToolResult.error(e.getMessage());
        } catch (Exception e) {
            ClaudeCodeView.debug("[tasks] failed: " + e);
            return McpToolResult.error("tasks failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }
}
