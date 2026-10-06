package com.anthropic.claudecode.eclipse.tools;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

import org.eclipse.ui.IViewReference;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.IWorkbenchPart;
import org.eclipse.ui.PlatformUI;
import org.eclipse.ui.views.IViewDescriptor;
import org.eclipse.ui.views.IViewRegistry;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCliView;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.anthropic.claudecode.eclipse.ui.ClaudeGuiView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Opens, closes and lists workbench views — what Window &gt; Show View and a view's close
 * button do. Views are looked up in the workbench's view registry, so any plug-in's views
 * are reachable by id and none is named here; the registry applies activity filtering, so a
 * view hidden from Show View is unknown to this tool as well.
 *
 * <p>Relies only on {@code org.eclipse.ui}, a hard {@code Require-Bundle} — no availability
 * guard.
 */
public class EclipseShowViewTool implements McpTool {

    /** Closing a view that hosts Claude sessions ends them, so the reply must leave first. */
    private static final int SESSION_VIEW_CLOSE_DELAY_MS = 500;

    @Override
    public String toolName() {
        return "eclipseShowView";
    }

    @Override
    public String description() {
        return "Open, close or list Eclipse views — the panels around the editor, such as Problems, "
                + "Console, Project Explorer or Git Staging (Window > Show View). action='list' "
                + "(default) shows the views this Eclipse has, with their ids and whether each is "
                + "open; narrow it with 'filter'. action='open' brings the view with 'viewId' to "
                + "the front, and action='close' closes it. Closing the Claude Code or Claude "
                + "Terminal view ends the sessions running in it.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");

        JsonObject props = new JsonObject();

        JsonObject action = new JsonObject();
        action.addProperty("type", "string");
        action.addProperty("description", "list (default), open, or close");
        JsonArray actions = new JsonArray();
        actions.add("list");
        actions.add("open");
        actions.add("close");
        action.add("enum", actions);
        props.add("action", action);

        JsonObject viewId = new JsonObject();
        viewId.addProperty("type", "string");
        viewId.addProperty("description",
                "The view's id, as shown by action='list', e.g. 'org.eclipse.ui.views.ProblemView'. "
                        + "Required for 'open' and 'close'.");
        props.add("viewId", viewId);

        JsonObject filter = new JsonObject();
        filter.addProperty("type", "string");
        filter.addProperty("description",
                "For 'list': only views whose id, name or category contains this text, e.g. 'git'.");
        props.add("filter", filter);

        JsonObject openOnly = new JsonObject();
        openOnly.addProperty("type", "boolean");
        openOnly.addProperty("description", "For 'list': only views that are open (default false).");
        props.add("openOnly", openOnly);

        JsonObject activate = new JsonObject();
        activate.addProperty("type", "boolean");
        activate.addProperty("description",
                "For 'open': also give the view keyboard focus (default false: it is brought to "
                        + "the front and focus stays where it is).");
        props.add("activate", activate);

        schema.add("properties", props);
        return schema;   // no args means "list"
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        try {
            String action = str(params, "action");
            String viewId = str(params, "viewId");
            ClaudeCodeView.debug("[eclipseShowView]action=" + action + " viewId=" + viewId);

            if (action == null || "list".equalsIgnoreCase(action)) {
                String filter = str(params, "filter");
                boolean openOnly = bool(params, "openOnly");
                return onUiThread(() -> list(filter, openOnly));
            }
            if ("open".equalsIgnoreCase(action)) {
                if (viewId == null) return McpToolResult.error("'viewId' is required for action='open'.");
                boolean activate = bool(params, "activate");
                return onUiThread(() -> open(viewId, activate));
            }
            if ("close".equalsIgnoreCase(action)) {
                if (viewId == null) return McpToolResult.error("'viewId' is required for action='close'.");
                return onUiThread(() -> close(viewId));
            }
            return McpToolResult.error("Unknown action: " + action + ". Use list, open or close.");
        } catch (Exception e) {
            return failed(e);
        }
    }

    // ── Actions ─────────────────────────────────────────────────────────────────────

    private McpToolResult list(String filter, boolean openOnly) {
        IWorkbenchPage page = UiHelper.getActivePage();
        if (page == null) return McpToolResult.error("No workbench window is open.");

        String needle = filter == null ? null : filter.toLowerCase(Locale.ROOT);
        JsonArray listed = new JsonArray();
        for (IViewDescriptor view : PlatformUI.getWorkbench().getViewRegistry().getViews()) {
            String[] categoryPath = view.getCategoryPath();
            String category = categoryPath == null ? "" : String.join("/", categoryPath);
            if (needle != null && !contains(view.getId(), needle) && !contains(view.getLabel(), needle)
                    && !contains(category, needle)) {
                continue;
            }
            List<IViewReference> refs = openReferences(page, view.getId());
            if (openOnly && refs.isEmpty()) continue;

            JsonObject j = new JsonObject();
            j.addProperty("id", view.getId());
            j.addProperty("name", view.getLabel());
            if (!category.isEmpty()) j.addProperty("category", category);
            j.addProperty("open", !refs.isEmpty());
            if (!refs.isEmpty()) j.addProperty("visible", anyVisible(page, refs));
            listed.add(j);
        }

        JsonObject out = new JsonObject();
        out.addProperty("count", listed.size());
        out.add("views", listed);
        ClaudeCodeView.debug("[eclipseShowView]list filter=" + filter + " openOnly=" + openOnly + " → "
                + listed.size());
        return McpToolResult.success(out);
    }

    private McpToolResult open(String viewId, boolean activate) throws Exception {
        IWorkbenchPage page = UiHelper.getActivePage();
        if (page == null) return McpToolResult.error("No workbench window is open.");

        IViewDescriptor view = PlatformUI.getWorkbench().getViewRegistry().find(viewId);
        if (view == null) return unknownView(viewId);

        page.showView(viewId, null, activate ? IWorkbenchPage.VIEW_ACTIVATE : IWorkbenchPage.VIEW_VISIBLE);

        JsonObject out = new JsonObject();
        out.addProperty("opened", true);
        out.addProperty("id", viewId);
        out.addProperty("name", view.getLabel());
        ClaudeCodeView.debug("[eclipseShowView]opened " + viewId + " activate=" + activate);
        return McpToolResult.success(out);
    }

    private McpToolResult close(String viewId) {
        IWorkbenchPage page = UiHelper.getActivePage();
        if (page == null) return McpToolResult.error("No workbench window is open.");

        List<IViewReference> refs = openReferences(page, viewId);
        if (refs.isEmpty()) {
            IViewRegistry registry = PlatformUI.getWorkbench().getViewRegistry();
            if (registry.find(viewId) == null) return unknownView(viewId);
            JsonObject out = new JsonObject();
            out.addProperty("closed", false);
            out.addProperty("id", viewId);
            out.addProperty("note", "The view was not open.");
            return McpToolResult.success(out);
        }

        JsonObject out = new JsonObject();
        out.addProperty("closed", true);
        out.addProperty("id", viewId);
        if (refs.size() > 1) out.addProperty("instances", refs.size());

        if (ClaudeGuiView.VIEW_ID.equals(viewId) || ClaudeCliView.VIEW_ID.equals(viewId)) {
            out.addProperty("note", "The view closes in a moment. The Claude sessions running in it "
                    + "end with it — including this one, if it runs there.");
            page.getWorkbenchWindow().getShell().getDisplay().timerExec(SESSION_VIEW_CLOSE_DELAY_MS,
                    () -> hide(page, refs));
        } else {
            hide(page, refs);
        }
        return McpToolResult.success(out);
    }

    // ── Helpers ─────────────────────────────────────────────────────────────────────

    private static void hide(IWorkbenchPage page, List<IViewReference> refs) {
        for (IViewReference ref : refs) {
            try {
                page.hideView(ref);
                ClaudeCodeView.debug("[eclipseShowView]closed " + ref.getId()
                        + (ref.getSecondaryId() == null ? "" : ":" + ref.getSecondaryId()));
            } catch (RuntimeException e) {
                ClaudeCodeView.debug("[eclipseShowView]close failed for " + ref.getId() + ": " + e);
            }
        }
    }

    /** Every open instance: {@code findViewReference(id)} misses those with a secondary id. */
    private static List<IViewReference> openReferences(IWorkbenchPage page, String viewId) {
        List<IViewReference> refs = new ArrayList<>();
        for (IViewReference ref : page.getViewReferences()) {
            if (viewId.equals(ref.getId())) refs.add(ref);
        }
        return refs;
    }

    private static boolean anyVisible(IWorkbenchPage page, List<IViewReference> refs) {
        for (IViewReference ref : refs) {
            // Null until the view is first shown: an open tab that was never brought to the front.
            IWorkbenchPart part = ref.getPart(false);
            if (part != null && page.isPartVisible(part)) return true;
        }
        return false;
    }

    private static McpToolResult unknownView(String viewId) {
        return McpToolResult.error("No view with id '" + viewId + "' in this Eclipse. Use "
                + "action='list', optionally with 'filter', to find the id.");
    }

    private interface UiAction {
        McpToolResult run() throws Exception;
    }

    private static McpToolResult onUiThread(UiAction action) {
        McpToolResult result = UiHelper.syncCall(() -> {
            try {
                return action.run();
            } catch (Exception e) {
                return failed(e);
            }
        });
        return result != null ? result : McpToolResult.error("The workbench is not available.");
    }

    private static McpToolResult failed(Exception e) {
        ClaudeCodeView.debug("[eclipseShowView]failed: " + e);
        return McpToolResult.error("eclipseShowView failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
    }

    private static boolean contains(String haystack, String needle) {
        return haystack != null && haystack.toLowerCase(Locale.ROOT).contains(needle);
    }

    private static boolean bool(JsonObject params, String key) {
        return params.has(key) && !params.get(key).isJsonNull() && params.get(key).getAsBoolean();
    }

    private static String str(JsonObject params, String key) {
        if (!params.has(key) || params.get(key).isJsonNull()) return null;
        String v = params.get(key).getAsString().trim();
        return v.isEmpty() ? null : v;
    }
}
