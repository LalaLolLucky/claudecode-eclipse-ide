package com.anthropic.claudecode.eclipse.tools;

import java.util.List;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCliView;
import com.anthropic.claudecode.eclipse.ui.TerminalTab;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * The Claude Terminal's part of {@link ClaudeCodeEclipseTool}: opening a terminal tab, and typing
 * a slash command into one.
 *
 * <p>Claude runs in that view as the terminal program it is, so there is no page to ask and no
 * list of commands to check one against: a command is typed at the prompt and Enter pressed, the
 * way the view itself renames a tab. The view works in the workspace folder only; it has no
 * folder tabs.
 */
class ClaudeTerminalModule implements ClaudeCodeEclipseTool.Module {

    private static final List<String> ACTIONS = List.of("newTab", "runCommand");

    /** The Claude Terminal view. Every call is safe from any thread. */
    interface Terminal {

        /** Its tabs, in the order they stand; none when the view is closed. */
        List<TerminalTab> tabs();

        /** Opens a tab, and the view first when it is closed. Its id, or null when none was opened. */
        String open();

        /** Types {@code text} into the tab and presses Enter. False when it could not be. */
        boolean submit(String tabId, String text);
    }

    /** The view itself, reached on the UI thread. */
    private static final Terminal VIEW = new Terminal() {
        @Override
        public List<TerminalTab> tabs() {
            List<TerminalTab> tabs = UiHelper.syncCall(ClaudeCliView::toolTabs);
            return tabs == null ? List.of() : tabs;
        }

        @Override
        public String open() {
            return UiHelper.syncCall(ClaudeCliView::toolOpenTab);
        }

        @Override
        public boolean submit(String tabId, String text) {
            return Boolean.TRUE.equals(UiHelper.syncCall(() -> ClaudeCliView.toolSubmit(tabId, text)));
        }
    };

    private final Terminal terminal;

    ClaudeTerminalModule() {
        this(VIEW);
    }

    ClaudeTerminalModule(Terminal terminal) {
        this.terminal = terminal;
    }

    @Override
    public String name() {
        return "claudeTerminal";
    }

    @Override
    public String description() {
        return "the Claude Terminal view, where Claude runs as its own terminal program, always in the "
                + "workspace folder. action='newTab' opens a new terminal tab, and the view when it is "
                + "closed, and brings it to the front. action='runCommand' types the slash command in "
                + "'command' into the terminal tab with 'tabId', or the one in front when it is left "
                + "out, and presses Enter, as if it were typed there: any command of Claude Code works, "
                + "and nothing it prints comes back. A command that opens a picker (/model, /resume, "
                + "/mcp, /config) leaves it waiting for keys in that terminal, where the next command "
                + "would be typed into it; and whatever is already typed at that prompt is submitted "
                + "along with the command. A tab just opened takes a moment to start. Every result "
                + "lists the terminal tabs with their ids.";
    }

    @Override
    public void describeParams(JsonObject props) {
        ClaudeCodeEclipseTool.param(props, "tabId", "string", "claudeTerminal runCommand: the terminal "
                + "tab, as every result of this module lists them, e.g. 'term2'. Left out, the terminal "
                + "tab in front.");
        ClaudeCodeEclipseTool.param(props, "command", "string", "claudeTerminal runCommand: the slash "
                + "command, on one line, with anything it takes after it.");
    }

    @Override
    public McpToolResult run(String action, JsonObject params) {
        if ("newTab".equals(action)) return newTab();
        if ("runCommand".equals(action)) return runCommand(params);
        return McpToolResult.error("Unknown action: " + action + ". Use one of: "
                + String.join(", ", ACTIONS) + ".");
    }

    private McpToolResult newTab() {
        String id = terminal.open();
        if (id == null) {
            return McpToolResult.error("The Claude Terminal did not open a new tab: it was still setting "
                    + "up the last one. Try again in a moment.");
        }
        List<TerminalTab> tabs = terminal.tabs();
        JsonObject out = new JsonObject();
        TerminalTab tab = find(tabs, id);
        out.add("tab", tab != null ? json(tab) : json(new TerminalTab(id, "", true, false, false)));
        out.add("tabs", json(tabs));
        return McpToolResult.success(out);
    }

    private McpToolResult runCommand(JsonObject params) {
        String command = ClaudeCodeEclipseTool.str(params, "command");
        String refusal = commandError(command);
        if (refusal != null) return McpToolResult.error(refusal);

        List<TerminalTab> tabs = terminal.tabs();
        if (tabs.isEmpty()) {
            return McpToolResult.error("No Claude Terminal tab is open. Open one with action 'newTab'.");
        }
        String tabId = ClaudeCodeEclipseTool.str(params, "tabId");
        TerminalTab tab = tabId == null ? front(tabs) : find(tabs, tabId);
        if (tab == null) {
            return McpToolResult.error((tabId == null ? "No Claude Terminal tab is in front."
                    : "No Claude Terminal tab '" + tabId + "'.") + " The tabs are: " + ids(tabs) + ".");
        }
        if (tab.ended()) {
            return McpToolResult.error("Claude has ended in that terminal tab. Open a new one with "
                    + "action 'newTab'.");
        }
        if (!tab.started()) {
            return McpToolResult.error("That terminal tab is still starting. Try again in a moment.");
        }
        String text = command.strip();
        if (!terminal.submit(tab.id(), text)) {
            return McpToolResult.error("The command could not be typed into that terminal tab.");
        }
        JsonObject out = new JsonObject();
        out.addProperty("typed", text);
        out.add("tab", json(tab));
        out.add("tabs", json(tabs));
        return McpToolResult.success(out);
    }

    /**
     * Why {@code command} cannot be typed into a terminal, or null when it can. It is typed as it
     * stands and Enter pressed after it, so it has to be one line of plain text: a line break in it
     * would submit it early and leave the rest to be read as a prompt, and Escape and the other
     * control characters are keys to the program running there.
     */
    static String commandError(String command) {
        String text = command == null ? "" : command.strip();
        if (text.length() < 2 || text.charAt(0) != '/' || Character.isWhitespace(text.charAt(1))) {
            return "'command' must be a slash command such as '/compact'.";
        }
        for (int i = 0; i < text.length(); i++) {
            if (Character.isISOControl(text.charAt(i))) {
                return "'command' must be one line of plain text: a line break would submit it early, "
                        + "and the other control characters are keys to the terminal.";
            }
        }
        return null;
    }

    private static TerminalTab find(List<TerminalTab> tabs, String id) {
        for (TerminalTab tab : tabs) if (tab.id().equals(id)) return tab;
        return null;
    }

    private static TerminalTab front(List<TerminalTab> tabs) {
        for (TerminalTab tab : tabs) if (tab.active()) return tab;
        return null;
    }

    private static String ids(List<TerminalTab> tabs) {
        StringBuilder out = new StringBuilder();
        for (TerminalTab tab : tabs) out.append(out.length() == 0 ? "" : ", ").append(tab.id());
        return out.toString();
    }

    private static JsonObject json(TerminalTab tab) {
        JsonObject o = new JsonObject();
        o.addProperty("id", tab.id());
        o.addProperty("title", tab.title());
        o.addProperty("active", tab.active());
        o.addProperty("started", tab.started());
        o.addProperty("ended", tab.ended());
        return o;
    }

    private static JsonArray json(List<TerminalTab> tabs) {
        JsonArray out = new JsonArray();
        for (TerminalTab tab : tabs) out.add(json(tab));
        return out;
    }
}
