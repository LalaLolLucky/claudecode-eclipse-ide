package com.anthropic.claudecode.eclipse.tools;

import org.eclipse.core.resources.IWorkspace;
import org.eclipse.core.resources.IWorkspaceDescription;
import org.eclipse.core.resources.ResourcesPlugin;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonObject;

/**
 * Reads or sets the workspace's automatic build — the tool form of Project &gt; Build
 * Automatically. It is a workspace setting, so it stays as set after the call and across
 * restarts, the same as when the menu item is used.
 *
 * <p>Uses only {@code org.eclipse.core.resources}, already a hard {@code Require-Bundle}.
 */
public class AutoBuildTool implements McpTool {

    @Override
    public String toolName() {
        return "autoBuild";
    }

    @Override
    public String description() {
        return "Report whether the workspace builds automatically (Project > Build Automatically), "
                + "or switch that on or off with 'enabled'. Without 'enabled' nothing is changed. "
                + "It is a workspace setting and stays as set. Switching it on lets Eclipse start "
                + "building whatever is out of date, in the background.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject props = new JsonObject();
        props.add("enabled", ToolSupport.prop("boolean",
                "true switches automatic building on, false switches it off. Leave out to only report it."));
        return ToolSupport.schema(props);
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        ClaudeCodeView.debug("[autoBuild] " + params);
        try {
            IWorkspace workspace = ResourcesPlugin.getWorkspace();
            IWorkspaceDescription description = workspace.getDescription();
            boolean was = description.isAutoBuilding();
            boolean now = ToolSupport.bool(params, "enabled", was);
            if (now != was) {
                description.setAutoBuilding(now);
                workspace.setDescription(description);
            }
            JsonObject result = new JsonObject();
            result.addProperty("autoBuildEnabled", now);
            result.addProperty("changed", now != was);
            return McpToolResult.success(result);
        } catch (Exception e) {
            ClaudeCodeView.debug("[autoBuild] failed: " + e);
            return McpToolResult.error("autoBuild failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }
}
