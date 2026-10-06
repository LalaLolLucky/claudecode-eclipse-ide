package com.anthropic.claudecode.eclipse.tools;

import java.util.Locale;

import org.eclipse.ui.IPerspectiveDescriptor;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.PlatformUI;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Opens, closes and lists workbench perspectives — what Window &gt; Perspective &gt; Open
 * Perspective and a perspective button's Close do. Perspectives are looked up in the
 * workbench's perspective registry, so any plug-in's perspectives are reachable by id and
 * none is named here.
 *
 * <p>Relies only on {@code org.eclipse.ui}, a hard {@code Require-Bundle} — no availability
 * guard.
 */
public class EclipseOpenPerspectiveTool implements McpTool {

    /** Closing the perspective on screen can close the view a Claude session runs in, so the
     *  reply must leave first. */
    private static final int ACTIVE_PERSPECTIVE_CLOSE_DELAY_MS = 500;

    @Override
    public String toolName() {
        return "eclipseOpenPerspective";
    }

    @Override
    public String description() {
        return "Open, close or list Eclipse perspectives — the saved window layouts such as Java, "
                + "Debug, Git or Plug-in Development (Window > Perspective > Open Perspective). "
                + "action='list' (default) shows the perspectives this Eclipse has, with their ids, "
                + "which are open and which one is active; narrow it with 'filter'. action='open' "
                + "switches the window to the perspective with 'perspectiveId', opening it if "
                + "needed. action='close' closes it; views that are open only in that perspective "
                + "close with it, and the last open perspective cannot be closed.";
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

        JsonObject perspectiveId = new JsonObject();
        perspectiveId.addProperty("type", "string");
        perspectiveId.addProperty("description",
                "The perspective's id, as shown by action='list', e.g. "
                        + "'org.eclipse.debug.ui.DebugPerspective'. Required for 'open' and 'close'.");
        props.add("perspectiveId", perspectiveId);

        JsonObject filter = new JsonObject();
        filter.addProperty("type", "string");
        filter.addProperty("description",
                "For 'list': only perspectives whose id or name contains this text, e.g. 'git'.");
        props.add("filter", filter);

        JsonObject openOnly = new JsonObject();
        openOnly.addProperty("type", "boolean");
        openOnly.addProperty("description",
                "For 'list': only perspectives that are open (default false).");
        props.add("openOnly", openOnly);

        schema.add("properties", props);
        return schema;   // no args means "list"
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        try {
            String action = str(params, "action");
            String perspectiveId = str(params, "perspectiveId");
            ClaudeCodeView.debug("[eclipseOpenPerspective] action=" + action + " perspectiveId="
                    + perspectiveId);

            if (action == null || "list".equalsIgnoreCase(action)) {
                String filter = str(params, "filter");
                boolean openOnly = bool(params, "openOnly");
                return onUiThread(() -> list(filter, openOnly));
            }
            if ("open".equalsIgnoreCase(action)) {
                if (perspectiveId == null) {
                    return McpToolResult.error("'perspectiveId' is required for action='open'.");
                }
                return onUiThread(() -> open(perspectiveId));
            }
            if ("close".equalsIgnoreCase(action)) {
                if (perspectiveId == null) {
                    return McpToolResult.error("'perspectiveId' is required for action='close'.");
                }
                return onUiThread(() -> close(perspectiveId));
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
        String activeId = idOf(page.getPerspective());
        JsonArray listed = new JsonArray();
        for (IPerspectiveDescriptor perspective
                : PlatformUI.getWorkbench().getPerspectiveRegistry().getPerspectives()) {
            if (needle != null && !contains(perspective.getId(), needle)
                    && !contains(perspective.getLabel(), needle)) {
                continue;
            }
            boolean open = isOpen(page, perspective.getId());
            if (openOnly && !open) continue;

            JsonObject j = new JsonObject();
            j.addProperty("id", perspective.getId());
            j.addProperty("name", perspective.getLabel());
            j.addProperty("open", open);
            if (perspective.getId().equals(activeId)) j.addProperty("active", true);
            listed.add(j);
        }

        JsonObject out = new JsonObject();
        if (activeId != null) out.addProperty("active", activeId);
        out.addProperty("count", listed.size());
        out.add("perspectives", listed);
        ClaudeCodeView.debug("[eclipseOpenPerspective] list filter=" + filter + " openOnly="
                + openOnly + " → " + listed.size());
        return McpToolResult.success(out);
    }

    private McpToolResult open(String perspectiveId) {
        IWorkbenchPage page = UiHelper.getActivePage();
        if (page == null) return McpToolResult.error("No workbench window is open.");

        IPerspectiveDescriptor perspective =
                PlatformUI.getWorkbench().getPerspectiveRegistry().findPerspectiveWithId(perspectiveId);
        if (perspective == null) return unknownPerspective(perspectiveId);

        String previous = idOf(page.getPerspective());
        page.setPerspective(perspective);

        JsonObject out = new JsonObject();
        out.addProperty("opened", true);
        out.addProperty("id", perspectiveId);
        out.addProperty("name", perspective.getLabel());
        if (previous != null && !previous.equals(perspectiveId)) out.addProperty("previous", previous);
        ClaudeCodeView.debug("[eclipseOpenPerspective] opened " + perspectiveId + " from " + previous);
        return McpToolResult.success(out);
    }

    private McpToolResult close(String perspectiveId) {
        IWorkbenchPage page = UiHelper.getActivePage();
        if (page == null) return McpToolResult.error("No workbench window is open.");

        IPerspectiveDescriptor perspective = null;
        IPerspectiveDescriptor[] open = page.getOpenPerspectives();
        for (IPerspectiveDescriptor candidate : open) {
            if (perspectiveId.equals(candidate.getId())) perspective = candidate;
        }
        if (perspective == null) {
            if (PlatformUI.getWorkbench().getPerspectiveRegistry().findPerspectiveWithId(perspectiveId)
                    == null) {
                return unknownPerspective(perspectiveId);
            }
            JsonObject out = new JsonObject();
            out.addProperty("closed", false);
            out.addProperty("id", perspectiveId);
            out.addProperty("note", "The perspective was not open.");
            return McpToolResult.success(out);
        }
        if (open.length == 1) {
            return McpToolResult.error("'" + perspectiveId + "' is the only open perspective. Open "
                    + "another one first: closing the last perspective closes every editor.");
        }

        JsonObject out = new JsonObject();
        out.addProperty("closed", true);
        out.addProperty("id", perspectiveId);

        IPerspectiveDescriptor target = perspective;
        if (perspectiveId.equals(idOf(page.getPerspective()))) {
            out.addProperty("note", "The perspective closes in a moment. Views open only in it close "
                    + "with it, and a Claude session running in such a view ends.");
            page.getWorkbenchWindow().getShell().getDisplay().timerExec(
                    ACTIVE_PERSPECTIVE_CLOSE_DELAY_MS, () -> closeNow(page, target));
        } else {
            closeNow(page, target);
        }
        return McpToolResult.success(out);
    }

    // ── Helpers ─────────────────────────────────────────────────────────────────────

    private static void closeNow(IWorkbenchPage page, IPerspectiveDescriptor perspective) {
        try {
            // Never the last perspective, so editors stay open and there is nothing to save.
            page.closePerspective(perspective, false, false);
            ClaudeCodeView.debug("[eclipseOpenPerspective] closed " + perspective.getId());
        } catch (RuntimeException e) {
            ClaudeCodeView.debug("[eclipseOpenPerspective] close failed for " + perspective.getId()
                    + ": " + e);
        }
    }

    private static boolean isOpen(IWorkbenchPage page, String perspectiveId) {
        for (IPerspectiveDescriptor open : page.getOpenPerspectives()) {
            if (perspectiveId.equals(open.getId())) return true;
        }
        return false;
    }

    private static String idOf(IPerspectiveDescriptor perspective) {
        return perspective == null ? null : perspective.getId();
    }

    private static McpToolResult unknownPerspective(String perspectiveId) {
        return McpToolResult.error("No perspective with id '" + perspectiveId + "' in this Eclipse. "
                + "Use action='list', optionally with 'filter', to find the id.");
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
        ClaudeCodeView.debug("[eclipseOpenPerspective] failed: " + e);
        return McpToolResult.error("eclipseOpenPerspective failed: " + e.getClass().getSimpleName()
                + ": " + e.getMessage());
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
