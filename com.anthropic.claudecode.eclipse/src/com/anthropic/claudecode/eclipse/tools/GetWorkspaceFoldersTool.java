package com.anthropic.claudecode.eclipse.tools;

import java.net.URI;

import org.eclipse.core.resources.IProject;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.IPath;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

public class GetWorkspaceFoldersTool implements McpTool {

    @Override
    public String toolName() {
        return "getWorkspaceFolders";
    }

    @Override
    public String description() {
        return "Get the list of workspace folders and open project paths in the Eclipse workspace.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");
        schema.add("properties", new JsonObject());
        return schema;
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        try {
            JsonArray folders = new JsonArray();

            for (IProject project : ResourcesPlugin.getWorkspace().getRoot().getProjects()) {
                JsonObject folder = describe(project);
                if (folder != null) folders.add(folder);
            }

            JsonObject result = new JsonObject();
            result.add("folders", folders);
            return McpToolResult.success(result);
        } catch (Exception e) {
            return McpToolResult.error("Failed to get workspace folders: " + e.getMessage());
        }
    }

    /**
     * One project as a folder entry, or null when it is closed or has no location. A
     * project kept on a file system other than the local one (#153) has a URI but no
     * disk path, so it is listed without {@code path}.
     */
    static JsonObject describe(IProject project) {
        if (!project.isOpen()) return null;
        URI uri = project.getLocationURI();
        if (uri == null) return null;
        JsonObject folder = new JsonObject();
        folder.addProperty("uri", uri.toString());
        folder.addProperty("name", project.getName());
        IPath location = project.getLocation();
        if (location != null) folder.addProperty("path", location.toOSString());
        return folder;
    }
}
