package com.anthropic.claudecode.eclipse.tools;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.regex.Pattern;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCliView;
import com.anthropic.claudecode.eclipse.ui.TerminalTab;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * The Claude Terminal's part of {@link ClaudeCodeEclipseTool}: opening and closing a terminal
 * tab, typing a prompt or a slash command into one, and what a command can set in one.
 *
 * <p>Claude runs in that view as the terminal program it is, so there is no page to ask, no
 * list of commands to check one against and nothing that reports back: whatever is done here
 * is typed at the prompt and Enter pressed, the way the view itself renames a tab. That is
 * also the limit of it — the model and the effort have a command that sets them, thinking and
 * the permission mode only keys that step to the next state, and Remote Control one command
 * that switches it on and, while it is on, shows a menu. The view works in the workspace
 * folder only; it has no folder tabs.
 */
class ClaudeTerminalModule implements ClaudeCodeEclipseTool.Module {

    private static final List<String> ACTIONS =
            List.of("newTab", "sendPrompt", "runCommand", "configureTab", "remoteControl", "closeTab");

    /** What the terminal's /effort takes. Which of the named levels a model has is the CLI's to
     *  say, and it says so in the terminal; 'auto' it always takes. */
    private static final List<String> EFFORTS = List.of("low", "medium", "high", "xhigh", "max", "auto");

    /** A model as /model takes one: an alias, a full id, with or without a context suffix. */
    private static final Pattern MODEL = Pattern.compile("[A-Za-z0-9][A-Za-z0-9._:\\[\\]-]{0,99}");

    private static final String NOT_REPORTED = "A terminal does not report Remote Control, so this is "
            + "what Eclipse last knew: a switch made by hand in that terminal is not seen.";

    /** The Claude Terminal view. Every call is safe from any thread. */
    interface Terminal {

        /** Its tabs, in the order they stand; none when the view is closed. */
        List<TerminalTab> tabs();

        /** Opens a tab, and the view first when it is closed. Its id, or null when none was opened. */
        String open();

        /** Types {@code text} into the tab and presses Enter. False when it could not be. */
        boolean submit(String tabId, String text);

        /**
         * Switches Remote Control for the tab by typing the CLI's own command — and, to switch
         * it off, answering the menu that command shows while it is on. False when the tab
         * could not take it.
         */
        boolean remoteControl(String tabId, boolean on);

        /** Closes the tab, ending Claude in it. False when there is no such tab. */
        boolean close(String tabId);
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

        @Override
        public boolean remoteControl(String tabId, boolean on) {
            return Boolean.TRUE.equals(UiHelper.syncCall(() -> ClaudeCliView.toolRemoteControl(tabId, on)));
        }

        @Override
        public boolean close(String tabId) {
            return Boolean.TRUE.equals(UiHelper.syncCall(() -> ClaudeCliView.toolClose(tabId)));
        }
    };

    /** A call that cannot be carried out, with the reason to answer it with. */
    private static final class Refusal extends RuntimeException {
        private static final long serialVersionUID = 1L;

        Refusal(String reason) {
            super(reason);
        }
    }

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
                + "workspace folder. Everything here is typed into a terminal tab and Enter pressed, "
                + "as if it were typed there, and nothing the terminal prints comes back; whatever is "
                + "already typed at that prompt is submitted along with it. The tab is the one with "
                + "'tabId', or the one in front when it is left out. action='newTab' opens a new "
                + "terminal tab, and the view when it is closed, and brings it to the front; it takes "
                + "a moment to start. action='sendPrompt' types 'prompt', which has to be one line. "
                + "action='runCommand' types the slash command in 'command': any command of Claude "
                + "Code works, and one that opens a picker (/model without a name, /resume, /mcp, "
                + "/config) leaves it waiting for keys in that terminal, where whatever is typed next "
                + "lands in it. action='configureTab' sets 'model', 'effort' or both with the "
                + "terminal's own /model and /effort; thinking and the permission mode cannot be set "
                + "there and are refused. action='remoteControl' switches Remote Control on or off "
                + "('enabled') with /remote-control, answering the terminal's menu a moment later to "
                + "switch it off. A terminal does not report Remote Control, so its state here is what "
                + "Eclipse last knew: if it was switched on by hand in that terminal, switching it on "
                + "from here opens its menu instead, and the next thing typed there would take "
                + "'Disconnect this session'. The first switch-on ever asks for a confirmation in the "
                + "terminal. action='closeTab' closes the tab with 'tabId', ending Claude in it; "
                + "closing the terminal tab the call came from ends this conversation. Every result "
                + "lists the terminal tabs with their ids.";
    }

    @Override
    public void describeParams(JsonObject props) {
        ClaudeCodeEclipseTool.param(props, "tabId", "string", "claudeTerminal sendPrompt/runCommand/"
                + "configureTab/remoteControl/closeTab: the terminal tab, as every result of this "
                + "module lists them, e.g. 'term2'. Left out, the terminal tab in front; 'closeTab' "
                + "requires it.");
        ClaudeCodeEclipseTool.param(props, "prompt", "string", "claudeTerminal sendPrompt: the message, "
                + "on one line.");
        ClaudeCodeEclipseTool.param(props, "command", "string", "claudeTerminal runCommand: the slash "
                + "command, on one line, with anything it takes after it.");
        ClaudeCodeEclipseTool.param(props, "model", "string", "claudeTerminal configureTab: a model as "
                + "the terminal's /model takes one, e.g. 'opus', 'sonnet[1m]' or a full id.");
        ClaudeCodeEclipseTool.param(props, "effort", "string", "claudeTerminal configureTab: low, "
                + "medium, high, xhigh, max or auto; which of the named ones a model has, the "
                + "terminal says.");
        ClaudeCodeEclipseTool.param(props, "enabled", "boolean", "claudeTerminal remoteControl: true "
                + "switches Remote Control on for the terminal tab, false switches it off.");
    }

    @Override
    public McpToolResult run(String action, JsonObject params) {
        try {
            switch (action) {
                case "newTab":
                    return newTab();
                case "sendPrompt":
                    return sendPrompt(params);
                case "runCommand":
                    return runCommand(params);
                case "configureTab":
                    return configureTab(params);
                case "remoteControl":
                    return remoteControl(params);
                case "closeTab":
                    return closeTab(params);
                default:
                    return McpToolResult.error("Unknown action: " + action + ". Use one of: "
                            + String.join(", ", ACTIONS) + ".");
            }
        } catch (Refusal refusal) {
            return McpToolResult.error(refusal.getMessage());
        }
    }

    private McpToolResult newTab() {
        String id = terminal.open();
        if (id == null) {
            throw new Refusal("The Claude Terminal did not open a new tab: it was still setting up the "
                    + "last one. Try again in a moment.");
        }
        List<TerminalTab> tabs = terminal.tabs();
        TerminalTab tab = find(tabs, id);
        JsonObject out = new JsonObject();
        out.add("tab", json(tab != null ? tab : new TerminalTab(id, "", true, false, false, "off")));
        out.add("tabs", json(tabs));
        return McpToolResult.success(out);
    }

    private McpToolResult sendPrompt(JsonObject params) {
        String prompt = ClaudeCodeEclipseTool.str(params, "prompt");
        String refusal = promptError(prompt);
        if (refusal != null) throw new Refusal(refusal);
        return typed(params, List.of(prompt.strip()));
    }

    private McpToolResult runCommand(JsonObject params) {
        String command = ClaudeCodeEclipseTool.str(params, "command");
        String refusal = commandError(command);
        if (refusal != null) throw new Refusal(refusal);
        return typed(params, List.of(command.strip()));
    }

    /** The model and the effort, each by the terminal's own command for it. */
    private McpToolResult configureTab(JsonObject params) {
        if (given(params, "thinking") || given(params, "mode")) {
            throw new Refusal("In the Claude Terminal, thinking is switched with a key that flips it and "
                    + "the permission mode with one that steps to the next. Only plan mode has a command "
                    + "(/plan), and a terminal does not report which mode it is in: typed while already "
                    + "planning, /plan shows the plan instead. So neither can be set from here; 'model' "
                    + "and 'effort' can.");
        }
        String model = ClaudeCodeEclipseTool.str(params, "model");
        String effort = ClaudeCodeEclipseTool.str(params, "effort");
        if (model == null && effort == null) {
            throw new Refusal("Nothing to change: give 'model', 'effort' or both.");
        }
        if (model != null && !MODEL.matcher(model).matches()) {
            throw new Refusal("'model' must be a model name such as 'opus' or 'sonnet[1m]', a full "
                    + "claude-… id, or 'default'.");
        }
        if (effort != null && !EFFORTS.contains(effort.toLowerCase(Locale.ROOT))) {
            throw new Refusal("Unknown effort '" + effort + "'. Use one of: " + String.join(", ", EFFORTS) + ".");
        }
        List<String> lines = new ArrayList<>();
        if (model != null) lines.add("/model " + model);
        if (effort != null) lines.add("/effort " + effort.toLowerCase(Locale.ROOT));
        return typed(params, lines);
    }

    /** Types each line into the tab the call is for, in order, and answers with what was typed. */
    private McpToolResult typed(JsonObject params, List<String> lines) {
        List<TerminalTab> tabs = terminal.tabs();
        TerminalTab tab = ready(tabFor(params, tabs));
        JsonArray typed = new JsonArray();
        for (String line : lines) {
            if (!terminal.submit(tab.id(), line)) {
                throw new Refusal("'" + line + "' could not be typed into that terminal tab.");
            }
            typed.add(line);
        }
        JsonObject out = new JsonObject();
        out.add("typed", typed);
        out.add("tab", json(tab));
        out.add("tabs", json(tabs));
        return McpToolResult.success(out);
    }

    private McpToolResult remoteControl(JsonObject params) {
        if (!given(params, "enabled") || !params.get("enabled").isJsonPrimitive()
                || !params.getAsJsonPrimitive("enabled").isBoolean()) {
            throw new Refusal("'enabled' must be true or false.");
        }
        boolean enabled = params.get("enabled").getAsBoolean();
        TerminalTab tab = ready(tabFor(params, terminal.tabs()));

        boolean switched = enabled != "on".equals(tab.remoteControl());
        if (switched && !terminal.remoteControl(tab.id(), enabled)) {
            throw new Refusal("Remote Control could not be switched in that terminal tab.");
        }
        List<TerminalTab> tabs = terminal.tabs();
        TerminalTab now = find(tabs, tab.id());
        JsonObject out = new JsonObject();
        out.addProperty("switched", switched);
        out.add("tab", json(now != null ? now : tab));
        out.add("tabs", json(tabs));
        JsonArray notes = new JsonArray();
        notes.add(NOT_REPORTED);
        out.add("notes", notes);
        return McpToolResult.success(out);
    }

    private McpToolResult closeTab(JsonObject params) {
        String tabId = ClaudeCodeEclipseTool.str(params, "tabId");
        if (tabId == null) {
            throw new Refusal("'tabId' is required: closing a terminal tab ends Claude in it.");
        }
        List<TerminalTab> tabs = terminal.tabs();
        if (find(tabs, tabId) == null) throw new Refusal(noSuchTab(tabId, tabs));
        if (!terminal.close(tabId)) throw new Refusal("That terminal tab could not be closed.");
        JsonObject out = new JsonObject();
        out.addProperty("closed", tabId);
        out.add("tabs", json(terminal.tabs()));
        return McpToolResult.success(out);
    }

    /** The tab a call is for: the one it names, or the one in front. */
    private static TerminalTab tabFor(JsonObject params, List<TerminalTab> tabs) {
        if (tabs.isEmpty()) {
            throw new Refusal("No Claude Terminal tab is open. Open one with action 'newTab'.");
        }
        String tabId = ClaudeCodeEclipseTool.str(params, "tabId");
        TerminalTab tab = tabId == null ? front(tabs) : find(tabs, tabId);
        if (tab == null) {
            throw new Refusal(tabId == null ? "No Claude Terminal tab is in front. The tabs are: " + ids(tabs) + "."
                    : noSuchTab(tabId, tabs));
        }
        return tab;
    }

    /** {@code tab}, once it is certain that something typed into it now reaches Claude's prompt. */
    private static TerminalTab ready(TerminalTab tab) {
        if (tab.ended()) {
            throw new Refusal("Claude has ended in that terminal tab. Open a new one with action 'newTab'.");
        }
        if (!tab.started()) {
            throw new Refusal("That terminal tab is still starting. Try again in a moment.");
        }
        if ("disconnecting".equals(tab.remoteControl())) {
            // Its menu is on screen, and what is typed now would land in the menu.
            throw new Refusal("That terminal tab is switching Remote Control off. Try again in a moment.");
        }
        return tab;
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
        return oneLine(text) ? null : "'command' must be one line of plain text: a line break would "
                + "submit it early, and the other control characters are keys to the terminal.";
    }

    /** Why {@code prompt} cannot be typed into a terminal as one message, or null when it can. */
    static String promptError(String prompt) {
        String text = prompt == null ? "" : prompt.strip();
        if (text.isEmpty()) return "'prompt' is required: the message to send.";
        if (text.charAt(0) == '/') return "A slash command is not sent as a message; use action 'runCommand'.";
        return oneLine(text) ? null : "'prompt' must be one line of plain text here: in a terminal a line "
                + "break submits what is typed so far, and the other control characters are keys to it.";
    }

    private static boolean oneLine(String text) {
        for (int i = 0; i < text.length(); i++) {
            if (Character.isISOControl(text.charAt(i))) return false;
        }
        return true;
    }

    private static boolean given(JsonObject params, String key) {
        return params.has(key) && !params.get(key).isJsonNull();
    }

    private static TerminalTab find(List<TerminalTab> tabs, String id) {
        for (TerminalTab tab : tabs) if (tab.id().equals(id)) return tab;
        return null;
    }

    private static TerminalTab front(List<TerminalTab> tabs) {
        for (TerminalTab tab : tabs) if (tab.active()) return tab;
        return null;
    }

    private static String noSuchTab(String tabId, List<TerminalTab> tabs) {
        return "No Claude Terminal tab '" + tabId + "'. The tabs are: " + ids(tabs) + ".";
    }

    private static String ids(List<TerminalTab> tabs) {
        StringBuilder out = new StringBuilder();
        for (TerminalTab tab : tabs) out.append(out.length() == 0 ? "" : ", ").append(tab.id());
        return out.length() == 0 ? "none" : out.toString();
    }

    private static JsonObject json(TerminalTab tab) {
        JsonObject o = new JsonObject();
        o.addProperty("id", tab.id());
        o.addProperty("title", tab.title());
        o.addProperty("active", tab.active());
        o.addProperty("started", tab.started());
        o.addProperty("ended", tab.ended());
        o.addProperty("remoteControl", tab.remoteControl());
        return o;
    }

    private static JsonArray json(List<TerminalTab> tabs) {
        JsonArray out = new JsonArray();
        for (TerminalTab tab : tabs) out.add(json(tab));
        return out;
    }
}
