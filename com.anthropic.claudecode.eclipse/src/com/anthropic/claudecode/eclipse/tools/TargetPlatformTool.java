package com.anthropic.claudecode.eclipse.tools;

import java.io.File;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.runtime.IStatus;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.core.runtime.Platform;
import org.eclipse.core.runtime.jobs.IJobChangeEvent;
import org.eclipse.core.runtime.jobs.JobChangeAdapter;
import org.osgi.framework.Bundle;
import org.osgi.framework.BundleContext;
import org.osgi.framework.FrameworkUtil;
import org.osgi.framework.ServiceReference;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * The target platform plug-in projects are built against (Preferences &gt; Plug-in
 * Development &gt; Target Platform): what it is, and reloading it — re-reading its
 * definition, resolving it and making the result the active target, which is what Reload
 * on that page does and what a changed {@code .target} file needs before a build sees it.
 *
 * <p>PDE is reached by name, through {@link ToolSupport#call}, and never imported: like
 * {@link RunAsTool} with PDE's launchers, an Eclipse without PDE then simply has no such
 * tool, and the plug-in builds against a target that has no PDE in it. Only PDE's API
 * types ({@code org.eclipse.pde.core.target}) are named.
 */
public class TargetPlatformTool implements McpTool {

    public static final String PDE = "org.eclipse.pde.core";

    private static final String API = "org.eclipse.pde.core.target.";
    private static final List<String> ACTIONS = List.of("show", "reload");

    /** How long 'reload' waits for PDE to finish making the target active. */
    private static final long LOAD_MINUTES = 10;

    @Override
    public String toolName() {
        return "targetPlatform";
    }

    @Override
    public String description() {
        return "Show or reload the target platform that plug-in projects are built against (Plug-in "
                + "Development > Target Platform). action='show' (default) names the active target, "
                + "where it is defined and what it is made of. action='reload' reads the active "
                + "target's definition again, resolves it and makes the result active, as Reload on "
                + "the preference page does; with 'file', it makes that .target file the active "
                + "target instead. Reload after a .target file has changed, then build. Resolving "
                + "can download from update sites and take minutes.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject props = new JsonObject();
        props.add("action", ToolSupport.choice("What to do (default 'show').", ACTIONS));
        props.add("file", ToolSupport.prop("string", "For 'reload': the absolute path of a .target file to "
                + "make the active target. Left out, the active target is reloaded."));
        return ToolSupport.schema(props);
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        ClaudeCodeView.debug("[targetPlatform] " + params);
        String action = ToolSupport.str(params, "action");
        if (action == null) action = "show";
        if (!ACTIONS.contains(action)) {
            return McpToolResult.error("Unknown action: " + action + ". Use one of: " + String.join(", ", ACTIONS) + ".");
        }
        BundleContext context = FrameworkUtil.getBundle(TargetPlatformTool.class).getBundleContext();
        ServiceReference<?> reference = null;
        try {
            Class<?> serviceType = ToolSupport.classOf(PDE, API + "ITargetPlatformService");
            reference = context.getServiceReference(serviceType.getName());
            if (reference == null) {
                // PDE registers the service when it starts, and it starts on first use.
                Platform.getBundle(PDE).start(Bundle.START_TRANSIENT);
                reference = context.getServiceReference(serviceType.getName());
            }
            Object service = reference == null ? null : context.getService(reference);
            if (service == null) return McpToolResult.error("PDE's target platform service is not available.");

            return action.equals("show")
                    ? McpToolResult.success(describe(ToolSupport.call(serviceType, service, "getWorkspaceTargetDefinition")))
                    : reload(serviceType, service, ToolSupport.str(params, "file"));
        } catch (Exception | LinkageError e) {
            ClaudeCodeView.debug("[targetPlatform] failed: " + e);
            return McpToolResult.error("targetPlatform failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        } finally {
            if (reference != null) context.ungetService(reference);
        }
    }

    private McpToolResult reload(Class<?> serviceType, Object service, String path) throws Exception {
        Object handle;
        if (path == null) {
            handle = ToolSupport.call(serviceType, service, "getWorkspaceTargetHandle");
            if (handle == null) {
                return McpToolResult.error("No target definition is active: plug-ins are built against "
                        + "the running Eclipse. Pass 'file' to make a .target file the active target.");
            }
        } else {
            File onDisk = new File(path);
            if (!onDisk.isFile()) return McpToolResult.error("No such file: " + path);
            IFile inWorkspace = ToolSupport.workspaceFile(path);
            handle = inWorkspace != null && inWorkspace.exists()
                    ? ToolSupport.call(serviceType, service, "getTarget", inWorkspace)
                    : ToolSupport.call(serviceType, service, "getTarget", onDisk.toURI());
        }
        Class<?> definitionType = ToolSupport.classOf(PDE, API + "ITargetDefinition");
        // A handle gives a new copy read from where the target is defined, which is the point.
        Object definition = ToolSupport.call(ToolSupport.classOf(PDE, API + "ITargetHandle"), handle, "getTargetDefinition");

        IStatus resolved = (IStatus) ToolSupport.call(definitionType, definition, "resolve", new NullProgressMonitor());
        if (resolved.getSeverity() == IStatus.ERROR || resolved.getSeverity() == IStatus.CANCEL) {
            return McpToolResult.error("The target could not be resolved, and the active target is "
                    + "unchanged: " + String.join("; ", messages(resolved)));
        }

        CountDownLatch loaded = new CountDownLatch(1);
        IStatus[] outcome = new IStatus[1];
        ToolSupport.call(ToolSupport.classOf(PDE, API + "LoadTargetDefinitionJob"), null, "load", definition,
                new JobChangeAdapter() {
                    @Override
                    public void done(IJobChangeEvent event) {
                        outcome[0] = event.getResult();
                        loaded.countDown();
                    }
                });
        boolean finished = loaded.await(LOAD_MINUTES, TimeUnit.MINUTES);

        JsonObject result = describe(definition);
        result.addProperty("reloaded", finished && outcome[0] != null && outcome[0].getSeverity() != IStatus.ERROR);
        JsonArray notes = new JsonArray();
        if (!resolved.isOK()) messages(resolved).forEach(notes::add);
        if (!finished) notes.add("PDE was still making the target active after " + LOAD_MINUTES + " minutes.");
        else if (outcome[0] != null && !outcome[0].isOK()) messages(outcome[0]).forEach(notes::add);
        if (!notes.isEmpty()) result.add("notes", notes);
        return McpToolResult.success(result);
    }

    private static JsonObject describe(Object definition) throws Exception {
        Class<?> definitionType = ToolSupport.classOf(PDE, API + "ITargetDefinition");
        JsonObject out = new JsonObject();
        out.addProperty("name", (String) ToolSupport.call(definitionType, definition, "getName"));
        Object handle = ToolSupport.call(definitionType, definition, "getHandle");
        if (handle != null) {
            // "resource:/project/x.target" for a workspace file, "local:…" for one kept by PDE.
            out.addProperty("definedIn", (String) ToolSupport.call(
                    ToolSupport.classOf(PDE, API + "ITargetHandle"), handle, "getMemento"));
        }
        JsonArray locations = new JsonArray();
        Object[] parts = (Object[]) ToolSupport.call(definitionType, definition, "getTargetLocations");
        if (parts != null) {
            Class<?> locationType = ToolSupport.classOf(PDE, API + "ITargetLocation");
            for (Object part : parts) {
                JsonObject location = new JsonObject();
                location.addProperty("type", (String) ToolSupport.call(locationType, part, "getType"));
                try {
                    location.addProperty("location", (String) ToolSupport.call(locationType, part, "getLocation", true));
                } catch (Exception e) {
                    location.addProperty("location", "(unreadable: " + e.getMessage() + ")");
                }
                locations.add(location);
            }
        }
        out.add("locations", locations);
        boolean resolved = (Boolean) ToolSupport.call(definitionType, definition, "isResolved");
        out.addProperty("resolved", resolved);
        if (resolved) {
            Object[] bundles = (Object[]) ToolSupport.call(definitionType, definition, "getBundles");
            out.addProperty("bundleCount", bundles == null ? 0 : bundles.length);
        }
        return out;
    }

    /** A status and whatever it is made of, as the lines a person would read; at most ten. */
    private static List<String> messages(IStatus status) {
        List<String> out = new java.util.ArrayList<>();
        if (status.getMessage() != null && !status.getMessage().isBlank()) out.add(status.getMessage());
        for (IStatus child : status.getChildren()) {
            if (out.size() >= 10) break;
            if (!child.isOK() && child.getMessage() != null && !out.contains(child.getMessage())) out.add(child.getMessage());
        }
        return out;
    }
}
