package com.anthropic.claudecode.eclipse.tools;

import java.io.File;

import org.eclipse.core.resources.IContainer;
import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.resources.IWorkspaceRoot;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.ltk.core.refactoring.Refactoring;
import org.eclipse.ltk.core.refactoring.RefactoringDescriptor;
import org.eclipse.ltk.core.refactoring.RefactoringStatus;
import org.eclipse.ltk.core.refactoring.resource.MoveResourcesDescriptor;
import org.eclipse.ltk.core.refactoring.resource.RenameResourceDescriptor;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Renames or moves a file, folder or project through Eclipse's refactoring engine — the
 * tool form of File › Rename and File › Move — instead of on the filesystem.
 *
 * <p>Doing it as a refactoring rather than a shell {@code mv} means: installed plug-ins take
 * part through their rename/move participants and update their own references to the
 * resource; editors, markers and the resource tree follow the change instead of going stale;
 * and the change is recorded with the refactoring undo manager, which is what Edit › Undo
 * offers after a refactoring. {@link RefactoringRunner} does the running: conditions are
 * checked first and the refactoring is refused, with nothing changed, when they report an
 * error — the point where the Rename dialog would stop and ask. Warnings are returned
 * alongside a successful result.
 *
 * <p>Uses {@code org.eclipse.ltk.core.refactoring} and {@code org.eclipse.core.resources},
 * both hard {@code Require-Bundle}s and both part of the base Eclipse Platform.
 */
public class RefactorResourceTool implements McpTool {

    @Override
    public String toolName() {
        return "refactorResource";
    }

    @Override
    public String description() {
        return "Rename or move a file, folder or project the way Eclipse's File > Rename / Move does, "
                + "as a refactoring: installed plug-ins update their own references to it (for "
                + "example PDE for plug-in files, CDT for #include lines of a moved header, and "
                + "language servers connected through LSP4E), open editors follow the change, and "
                + "it can be undone with Edit > Undo. Prefer this to a shell mv for "
                + "anything inside the workspace. For renaming or moving a Java type, method or "
                + "package with its references in code, use 'refactorJava' instead when available.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");

        JsonObject props = new JsonObject();

        JsonObject action = new JsonObject();
        action.addProperty("type", "string");
        JsonArray actions = new JsonArray();
        actions.add("rename");
        actions.add("move");
        action.add("enum", actions);
        action.addProperty("description", "rename (needs 'newName') or move (needs 'destination').");
        props.add("action", action);

        props.add("path", prop("string", "Absolute path of the file, folder or project."));
        props.add("newName", prop("string", "For rename: the new name (a name, not a path)."));
        props.add("destination", prop("string", "For move: absolute path of an existing folder "
                + "or project in the workspace to move into."));
        props.add("updateReferences", prop("boolean", "Let participants update references to the "
                + "resource (default true)."));

        schema.add("properties", props);
        JsonArray required = new JsonArray();
        required.add("action");
        required.add("path");
        schema.add("required", required);
        return schema;
    }

    private static JsonObject prop(String type, String description) {
        JsonObject p = new JsonObject();
        p.addProperty("type", type);
        p.addProperty("description", description);
        return p;
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        try {
            String action = EditorUtils.getString(params, "action", "action");
            String path = EditorUtils.getString(params, "path", "path");
            if (action == null || path == null) {
                return McpToolResult.error("Missing required parameters: action and path.");
            }
            boolean updateReferences = !params.has("updateReferences")
                    || params.get("updateReferences").isJsonNull()
                    || params.get("updateReferences").getAsBoolean();

            IResource resource = resourceAt(path);
            if (resource == null) return McpToolResult.error("Not in the workspace: " + path);

            RefactoringDescriptor descriptor;
            String expected;
            if ("rename".equals(action)) {
                String newName = EditorUtils.getString(params, "newName", "newName");
                if (newName == null || newName.isBlank() || newName.contains("/") || newName.contains("\\")) {
                    return McpToolResult.error("action='rename' needs 'newName': a name, not a path.");
                }
                RenameResourceDescriptor rename = new RenameResourceDescriptor();
                rename.setResourcePath(resource.getFullPath());
                rename.setNewName(newName);
                rename.setUpdateReferences(updateReferences);
                descriptor = rename;
                expected = resource.getFullPath().removeLastSegments(1).append(newName).toString();
            } else if ("move".equals(action)) {
                String destinationPath = EditorUtils.getString(params, "destination", "destination");
                IResource destination = destinationPath == null ? null : resourceAt(destinationPath);
                if (!(destination instanceof IContainer container)) {
                    return McpToolResult.error("action='move' needs 'destination': an existing folder or "
                            + "project in the workspace.");
                }
                MoveResourcesDescriptor move = new MoveResourcesDescriptor();
                move.setResourcesToMove(new IResource[] { resource });
                move.setDestination(container);
                move.setUpdateReferences(updateReferences);
                descriptor = move;
                expected = container.getFullPath().append(resource.getName()).toString();
            } else {
                return McpToolResult.error("Unknown action: " + action + ". Use rename or move.");
            }

            RefactoringStatus createStatus = new RefactoringStatus();
            Refactoring refactoring = descriptor.createRefactoring(createStatus);
            if (refactoring == null || createStatus.hasFatalError()) {
                return McpToolResult.error(RefactoringRunner.describe(
                        "The refactoring could not be created", createStatus));
            }

            RefactoringRunner.Outcome outcome = RefactoringRunner.run(refactoring, false);
            if (!outcome.performed()) {
                return McpToolResult.error(RefactoringRunner.describe("Refused — nothing was changed",
                        outcome.conditions(), outcome.validation()));
            }
            RefactoringStatus conditions = outcome.conditions();

            JsonObject out = new JsonObject();
            out.addProperty("action", action);
            out.addProperty("from", resource.getFullPath().toString());
            out.addProperty("to", expected);
            IResource moved = ResourcesPlugin.getWorkspace().getRoot().findMember(expected);
            if (moved != null && moved.getLocation() != null) {
                out.addProperty("location", moved.getLocation().toOSString());
            }
            JsonArray warnings = RefactoringRunner.messages(conditions);
            if (!warnings.isEmpty()) out.add("warnings", warnings);
            ClaudeCodeView.debug("[refactorResource] " + action + " " + resource.getFullPath() + " → " + expected);
            return McpToolResult.success(out);
        } catch (Exception e) {
            ClaudeCodeView.debug("[refactorResource] failed: " + e);
            return McpToolResult.error("refactorResource failed: " + e.getClass().getSimpleName()
                    + ": " + e.getMessage());
        }
    }

    /** The workspace resource at an absolute filesystem path: file, folder or project. */
    private static IResource resourceAt(String path) {
        IWorkspaceRoot root = ResourcesPlugin.getWorkspace().getRoot();
        File f = new File(path);
        if (f.isDirectory()) {
            for (IContainer c : root.findContainersForLocationURI(f.toURI())) {
                if (c.exists() && c.getType() != IResource.ROOT) return c;
            }
            return null;
        }
        for (IFile file : root.findFilesForLocationURI(f.toURI())) {
            if (file.exists()) return file;
        }
        return null;
    }
}
