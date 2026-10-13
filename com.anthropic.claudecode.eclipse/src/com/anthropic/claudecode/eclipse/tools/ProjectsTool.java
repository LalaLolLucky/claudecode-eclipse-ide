package com.anthropic.claudecode.eclipse.tools;

import java.io.File;
import java.util.ArrayList;
import java.util.List;

import org.eclipse.core.resources.IProject;
import org.eclipse.core.resources.IProjectDescription;
import org.eclipse.core.resources.IWorkspace;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.core.runtime.Path;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * The workspace's projects: which there are, open or closed, and opening, closing and
 * importing one — the tool form of the Project Explorer's Open Project and Close Project,
 * and of File &gt; Import &gt; Existing Projects into Workspace for a single folder.
 *
 * <p>It does not delete: removing a project from the workspace, with or without its files,
 * is left to the user.
 *
 * <p>Uses only {@code org.eclipse.core.resources}, already a hard {@code Require-Bundle}.
 */
public class ProjectsTool implements McpTool {

    private static final List<String> ACTIONS = List.of("list", "open", "close", "import");

    @Override
    public String toolName() {
        return "projects";
    }

    @Override
    public String description() {
        return "List, open, close or import Eclipse projects. action='list' (default) shows every "
                + "project of the workspace, closed ones included, with its folder and natures. "
                + "action='open' and action='close' take 'projects'; a closed project is left out of "
                + "builds, searches and the other tools until it is opened again. action='import' "
                + "adds the folder at 'path' to the workspace as a project without copying it: with "
                + "its own .project file when it has one, otherwise as a new plain project named "
                + "'name' or after the folder.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject props = new JsonObject();
        props.add("action", ToolSupport.choice("What to do (default 'list').", ACTIONS));
        props.add("projects", ToolSupport.strings("For 'open' / 'close': the project names."));
        props.add("path", ToolSupport.prop("string", "For 'import': the absolute path of the project's folder."));
        props.add("name", ToolSupport.prop("string", "For 'import' of a folder with no .project file: "
                + "the project's name. Left out, the folder's name."));
        return ToolSupport.schema(props);
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        ClaudeCodeView.debug("[projects] " + params);
        try {
            String action = ToolSupport.str(params, "action");
            if (action == null) action = "list";
            return switch (action) {
                case "list" -> list();
                case "open", "close" -> openOrClose(params, action.equals("open"));
                case "import" -> importProject(params);
                default -> McpToolResult.error("Unknown action: " + action + ". Use one of: "
                        + String.join(", ", ACTIONS) + ".");
            };
        } catch (Exception e) {
            ClaudeCodeView.debug("[projects] failed: " + e);
            return McpToolResult.error("projects failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }

    private McpToolResult list() throws Exception {
        JsonArray projects = new JsonArray();
        for (IProject p : ResourcesPlugin.getWorkspace().getRoot().getProjects()) projects.add(describe(p));
        JsonObject result = new JsonObject();
        result.add("projects", projects);
        return McpToolResult.success(result);
    }

    private McpToolResult openOrClose(JsonObject params, boolean open) throws Exception {
        List<String> names = ToolSupport.list(params, "projects");
        if (names.isEmpty()) return McpToolResult.error("'projects' is required: the names of the projects.");
        List<IProject> targets = new ArrayList<>();
        List<String> missing = new ArrayList<>();
        for (String name : names) {
            IProject p = ResourcesPlugin.getWorkspace().getRoot().getProject(name);
            if (p.exists()) targets.add(p);
            else missing.add(name);
        }
        if (!missing.isEmpty()) return McpToolResult.error("No such project: " + String.join(", ", missing));

        JsonArray changed = new JsonArray();
        for (IProject p : targets) {
            if (p.isOpen() == open) continue;
            if (open) p.open(new NullProgressMonitor());
            else p.close(new NullProgressMonitor());
            changed.add(p.getName());
        }
        JsonObject result = new JsonObject();
        result.add(open ? "opened" : "closed", changed);
        JsonArray projects = new JsonArray();
        for (IProject p : targets) projects.add(describe(p));
        result.add("projects", projects);
        return McpToolResult.success(result);
    }

    private McpToolResult importProject(JsonObject params) throws Exception {
        String path = ToolSupport.str(params, "path");
        if (path == null) return McpToolResult.error("'path' is required: the absolute path of the project's folder.");
        File folder = new File(path);
        if (!folder.isAbsolute() || !folder.isDirectory()) {
            return McpToolResult.error("Not a folder, or not an absolute path: " + path);
        }
        IWorkspace workspace = ResourcesPlugin.getWorkspace();
        IPath location = new Path(folder.getAbsolutePath());
        for (IProject p : workspace.getRoot().getProjects()) {
            if (location.equals(p.getLocation())) {
                return McpToolResult.error("That folder is already the project '" + p.getName() + "'"
                        + (p.isOpen() ? "." : ", which is closed; use action 'open'."));
            }
        }

        boolean described = new File(folder, IProjectDescription.DESCRIPTION_FILE_NAME).isFile();
        IProjectDescription description;
        if (described) {
            description = workspace.loadProjectDescription(location.append(IProjectDescription.DESCRIPTION_FILE_NAME));
        } else {
            String name = ToolSupport.str(params, "name");
            description = workspace.newProjectDescription(name == null ? folder.getName() : name);
        }
        // A folder directly inside the workspace folder is where a project of that name lives
        // by default, and Eclipse refuses that place spelled out as a location.
        boolean inWorkspaceFolder = workspace.getRoot().getLocation().equals(location.removeLastSegments(1));
        if (inWorkspaceFolder && !description.getName().equals(folder.getName())) {
            return McpToolResult.error("A project in the workspace folder has to be named after its "
                    + "folder, '" + folder.getName() + "', not '" + description.getName() + "'.");
        }
        description.setLocation(inWorkspaceFolder ? null : location);

        IProject project = workspace.getRoot().getProject(description.getName());
        if (project.exists()) {
            return McpToolResult.error("The workspace already has a project named '" + project.getName()
                    + "', in " + project.getLocation() + ".");
        }
        project.create(description, new NullProgressMonitor());
        project.open(new NullProgressMonitor());

        JsonObject result = new JsonObject();
        result.addProperty("imported", project.getName());
        result.addProperty("hadProjectFile", described);
        result.add("project", describe(project));
        return McpToolResult.success(result);
    }

    private static JsonObject describe(IProject p) throws Exception {
        JsonObject j = new JsonObject();
        j.addProperty("name", p.getName());
        j.addProperty("open", p.isOpen());
        if (p.getLocation() != null) j.addProperty("folder", p.getLocation().toOSString());
        if (p.isOpen()) {
            JsonArray natures = new JsonArray();
            for (String nature : p.getDescription().getNatureIds()) natures.add(nature);
            j.add("natures", natures);
        }
        return j;
    }
}
