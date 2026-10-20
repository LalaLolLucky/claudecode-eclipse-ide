package com.anthropic.claudecode.eclipse.tools;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.anthropic.claudecode.eclipse.ui.DebugModeUi;
import com.google.gson.JsonObject;

/**
 * The Claude IDE Server view's part of {@link ClaudeCodeEclipseTool}: reading its log. The view
 * is the plug-in's own diagnostic console, shown in Debug mode; its Copy Logs and Clear Logs
 * buttons are the user's, so nothing here changes it.
 */
class ClaudeIdeServerModule implements ClaudeCodeEclipseTool.Module {

    /** The Claude IDE Server view. Every call is safe from any thread. */
    interface Server {

        /** What the view shows, then the lines kept for it while it could not show them. */
        String log();

        /** Whether the view is open. */
        boolean open();

        /** Whether Debug mode is on. */
        boolean debugMode();
    }

    /** The view itself, read on the UI thread. */
    private static final Server VIEW = new Server() {
        @Override
        public String log() {
            String log = UiHelper.syncCall(ClaudeCodeView::toolLog);
            return log == null ? "" : log;
        }

        @Override
        public boolean open() {
            return Boolean.TRUE.equals(UiHelper.syncCall(ClaudeCodeView::toolViewOpen));
        }

        @Override
        public boolean debugMode() {
            return DebugModeUi.isDebugEnabled();
        }
    };

    private final Server server;

    ClaudeIdeServerModule() {
        this(VIEW);
    }

    ClaudeIdeServerModule(Server server) {
        this.server = server;
    }

    @Override
    public String name() {
        return "claudeIdeServer";
    }

    @Override
    public String description() {
        return "the Claude IDE Server view, the plug-in's own diagnostic log, shown while Debug mode "
                + "is on. action='readLog' returns its log as text: what the view shows, then any lines "
                + "kept for it while it could not show them (Debug mode keeps them from startup until the "
                + "view is first opened, and while it is closed); with 'lines', only the last that many. "
                + "It changes nothing in the view.";
    }

    @Override
    public void describeParams(JsonObject props) {
        ClaudeCodeEclipseTool.param(props, "lines", "integer", "claudeIdeServer readLog: how many of the "
                + "last lines to return; the whole log when left out.");
    }

    @Override
    public McpToolResult run(String action, JsonObject params) {
        if (!"readLog".equals(action)) {
            return McpToolResult.error("Unknown action for claudeIdeServer: " + action + ". Use: readLog.");
        }
        String log = server.log();
        int lines = ToolSupport.integer(params, "lines", 0, 0, Integer.MAX_VALUE);
        if (lines > 0) log = lastLines(log, lines);

        JsonObject out = new JsonObject();
        out.addProperty("log", log);
        out.addProperty("viewOpen", server.open());
        out.addProperty("debugMode", server.debugMode());
        if (log.isEmpty() && !server.debugMode()) {
            out.addProperty("note", "Debug mode is off, so nothing is logged here: it is switched on in "
                    + "the Claude Code preferences.");
        }
        return McpToolResult.success(out);
    }

    /** The last {@code n} lines of {@code text}, a final newline kept with its line. */
    static String lastLines(String text, int n) {
        int end = text.endsWith("\n") ? text.length() - 1 : text.length();
        int start = end;
        for (int found = 0; found < n; found++) {
            int nl = text.lastIndexOf('\n', start - 1);
            if (nl < 0) return text;
            start = nl;
        }
        return text.substring(start + 1);
    }
}
