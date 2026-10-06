package com.anthropic.claudecode.eclipse.tools;

import java.nio.file.InvalidPathException;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.List;
import java.util.Locale;
import java.util.function.BooleanSupplier;
import java.util.function.Function;
import java.util.regex.Pattern;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.anthropic.claudecode.eclipse.ui.ClaudeGuiView;
import com.anthropic.claudecode.eclipse.ui.TerminalOnlyUi;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * The Claude Code view's part of {@link ClaudeCodeEclipseTool}: its conversation tabs — opening
 * and closing them, sending a prompt or a slash command into one, switching Remote Control on or
 * off for one — each tab's model, effort, thinking and permission mode, and the saved
 * conversations a tab can be opened on.
 *
 * <p>The tabs and their settings live in the view's page, so an action on them is carried out
 * there ({@code scripts/claudetool.js}) by the same code paths the composer's own controls
 * use; this class only checks the action, passes its parameters across and turns the page's
 * reply into a tool result. The saved conversations are on disk, and are listed from here.
 */
class ClaudeCodeViewModule implements ClaudeCodeEclipseTool.Module {

    private static final List<String> ACTIONS = List.of("listTabs", "newTab", "configureTab", "sendPrompt", "runCommand", "remoteControl", "closeTab", "listHistory", "openSession");

    /** What an action may send to the page; anything else in the call stays behind. */
    private static final List<String> PARAMS = List.of("tabId", "folder", "model", "effort", "thinking", "mode", "prompt", "command", "enabled", "remoteControl");

    /** How many conversations 'listHistory' shows when no 'limit' says, and the most there are
     *  to show: the core lists a folder's 100 most recent. */
    private static final int LISTED = 30;
    private static final int MOST_LISTED = 100;

    /** What a conversation id may be. It becomes a file name. */
    private static final Pattern SESSION_ID = Pattern.compile("[A-Za-z0-9][A-Za-z0-9_-]{0,127}");

    /** What Claude Code wraps around a message that is not the user's words: the editor's
     *  context, and the commands' own bookkeeping. The core takes most of these out of a
     *  title and the history panel the rest (history.js stripMeta); the same ones go here. */
    private static final Pattern IDE_BLOCK = Pattern.compile("<(ide_[a-z_]*)\\b[^>]*>.*?</\\1>",
            Pattern.CASE_INSENSITIVE | Pattern.DOTALL);
    private static final Pattern IDE_EMPTY = Pattern.compile("<ide_[a-z_]*\\b[^>]*/>", Pattern.CASE_INSENSITIVE);
    private static final Pattern META_BLOCK = Pattern.compile("<(local-command-caveat|command-message|command-args"
            + "|local-command-stdout|command-stdout|command-contents|system-reminder)>.*?</\\1>",
            Pattern.CASE_INSENSITIVE | Pattern.DOTALL);
    private static final Pattern COMMAND_NAME = Pattern.compile("<command-name>(.*?)</command-name>",
            Pattern.CASE_INSENSITIVE | Pattern.DOTALL);
    private static final String UNTITLED = "(untitled)";

    /** The saved conversations of a folder. Asked without the view's page: scanning them can
     *  take a moment, and the page answers on the UI thread. */
    interface History {

        /** The folder meant when a call names none: the one in front in the view, or the
         *  workspace folder when the view is not open. */
        String frontFolder();

        /** A folder's conversations as the history panel lists them, newest first: a JSON
         *  array of {@code {sessionId, display, timestamp}}. */
        String list(String folder);

        /** Whether the folder has a conversation with this id, listed or too old to be. */
        boolean has(String folder, String sessionId);
    }

    private static final History VIEW_HISTORY = new History() {
        @Override
        public String frontFolder() {
            return ClaudeGuiView.frontFolder();
        }

        @Override
        public String list(String folder) {
            return ClaudeGuiView.sessionListOf(folder);
        }

        @Override
        public boolean has(String folder, String sessionId) {
            return ClaudeGuiView.hasSession(folder, sessionId);
        }
    };

    /** Asks the page: request JSON in, reply JSON out, null when there is no page to ask. */
    private final Function<String, String> page;

    /** Whether the view is switched off altogether ("Exclusively use terminal"). */
    private final BooleanSupplier viewOff;

    private final History history;

    ClaudeCodeViewModule() {
        this(ClaudeCodeViewModule::askView, TerminalOnlyUi::isOn, VIEW_HISTORY);
    }

    ClaudeCodeViewModule(Function<String, String> page) {
        this(page, () -> false);
    }

    ClaudeCodeViewModule(Function<String, String> page, BooleanSupplier viewOff) {
        this(page, viewOff, VIEW_HISTORY);
    }

    ClaudeCodeViewModule(Function<String, String> page, BooleanSupplier viewOff, History history) {
        this.page = page;
        this.viewOff = viewOff;
        this.history = history;
    }

    @Override
    public String name() {
        return "claudeCodeView";
    }

    @Override
    public String description() {
        return "the Claude Code chat view and its conversation tabs. action='listTabs' shows every "
                + "tab with its id, title, folder ('root'), model, effort, thinking, permission mode and Remote Control "
                + "state (with its claude.ai address once on), and the "
                + "models, efforts and modes a tab can be given, and the slash commands of the view. "
                + "action='newTab' opens a new "
                + "conversation tab and brings it to the front, on the defaults unless 'model', "
                + "'effort', 'thinking' or 'mode' say otherwise. It opens under the folder tab in front, "
                + "or under 'folder': a folder tab that is already open, or a folder the user has "
                + "trusted before, which gets a folder tab of its own. With 'prompt' it also sends that "
                + "as the tab's first message, and with 'remoteControl' true the tab comes up "
                + "reachable from the user's other devices. action='configureTab' changes "
                + "those settings on the tab with 'tabId', or on the tab in front when it is left "
                + "out; settings left out keep their value, and a running conversation takes the "
                + "change at once. action='sendPrompt' sends 'prompt' as the user's message in the "
                + "tab with 'tabId' and returns at once, without waiting for the reply; a tab "
                + "already working queues it, and 'listTabs' shows 'streaming' while a tab works. "
                + "action='runCommand' runs the slash command in 'command' in the tab with 'tabId'. "
                + "/compact, /clear, /context, /model, /remote-control and /help are carried out for "
                + "that tab and answered in the result; /clear replaces that tab's conversation with "
                + "a new one, which on the tab the call came from ends this conversation, and on a "
                + "tab under Remote Control cuts the user's other devices off from it. /rewind, "
                + "/resume, /mcp and /advisor open something in the view that waits for the user "
                + "there, and are refused. Any other command (Claude's own, a custom command, a "
                + "skill) is passed to Claude in that tab, and the call returns at once without "
                + "what it prints. "
                + "action='remoteControl' switches Remote Control on or off ('enabled') for the tab "
                + "with 'tabId'; it connects in the background, so 'listTabs' shows 'connecting' "
                + "and then 'on', and switching it off for the tab the user is talking through "
                + "from another device cuts them off from it. "
                + "action='closeTab' closes the tab with 'tabId', stopping whatever it is doing. "
                + "action='listHistory' lists the saved conversations of the folder tab in front, or "
                + "of 'folder', newest first, each with its id and title; 'query' keeps those whose "
                + "title contains it and 'limit' says how many to show. They are the ones on this "
                + "machine, not the ones on claude.ai. action='openSession' opens the conversation "
                + "with 'sessionId' in a new tab under that folder, on the settings it was last "
                + "used with, or brings its tab to the front when it is already open. "
                + "A call cannot tell which tab it was made from, so take 'tabId' from 'listTabs' "
                + "or 'newTab'; closing the tab the call came from ends this conversation.";
    }

    @Override
    public void describeParams(JsonObject props) {
        props.add("tabId", param("string", "claudeCodeView configureTab/sendPrompt/runCommand/remoteControl/closeTab: "
                + "the tab, as shown by 'listTabs', e.g. 'tab2'. Required for all but 'configureTab'; "
                + "'configureTab' without it changes the tab in front."));
        props.add("folder", param("string", "claudeCodeView newTab/listHistory/openSession: the full "
                + "path of the folder the conversation runs in, such as a 'root' from 'listTabs'. Left "
                + "out, the folder tab in front."));
        props.add("sessionId", param("string", "claudeCodeView openSession: the conversation, as "
                + "'listHistory' shows it."));
        props.add("query", param("string", "claudeCodeView listHistory: show only the conversations "
                + "whose title contains this."));
        props.add("limit", param("number", "claudeCodeView listHistory: how many to show, 1 to 100; "
                + "30 when left out."));
        props.add("prompt", param("string", "claudeCodeView sendPrompt/newTab: the message to send. "
                + "A slash command goes through 'runCommand' instead."));
        props.add("command", param("string", "claudeCodeView runCommand: the slash command, with "
                + "anything it takes after it, e.g. '/compact' or '/model opus'. 'listTabs' shows the "
                + "view's own."));
        props.add("model", param("string", "claudeCodeView newTab/configureTab: a model id from "
                + "'listTabs' (e.g. 'opus'), a full id such as 'claude-opus-5-5', or 'default'."));
        props.add("effort", param("string", "claudeCodeView newTab/configureTab: low, medium, high, "
                + "xhigh or max."));
        props.add("thinking", param("boolean", "claudeCodeView newTab/configureTab: extended thinking "
                + "on or off."));
        props.add("mode", param("string", "claudeCodeView newTab/configureTab: the permission mode — "
                + "default (ask before edits), acceptEdits, plan, auto or bypassPermissions; "
                + "'listTabs' shows which are available here."));
        props.add("enabled", param("boolean", "claudeCodeView remoteControl: true switches Remote "
                + "Control on for the tab, false switches it off."));
        props.add("remoteControl", param("boolean", "claudeCodeView newTab: true opens the tab with "
                + "Remote Control on. A 'prompt' given with it is sent once it has connected."));
    }

    @Override
    public McpToolResult run(String action, JsonObject params) {
        if (!ACTIONS.contains(action)) {
            return McpToolResult.error("Unknown action: " + action + ". Use one of: "
                    + String.join(", ", ACTIONS) + ".");
        }
        if (viewOff.getAsBoolean()) {
            return McpToolResult.error("The Claude Code view is switched off in this Eclipse: the "
                    + "\"Exclusively use terminal\" preference is on, so there are no tabs to act on.");
        }
        if ("listHistory".equals(action)) return listHistory(params);

        JsonObject request = new JsonObject();
        request.addProperty("action", action);
        for (String key : PARAMS) {
            if (params.has(key) && !params.get(key).isJsonNull()) request.add(key, params.get(key));
        }
        if ("openSession".equals(action)) {
            String refusal = describeSession(params, request);
            if (refusal != null) return McpToolResult.error(refusal);
        }

        String reply = page.apply(request.toString());
        if (reply == null) {
            return McpToolResult.error("The Claude Code view is not open. Open it with eclipseShowView "
                    + "(viewId '" + ClaudeGuiView.VIEW_ID + "') and try again.");
        }
        try {
            JsonObject out = JsonParser.parseString(reply).getAsJsonObject();
            if (!out.has("ok") || !out.get("ok").getAsBoolean()) {
                return McpToolResult.error(out.has("error") ? out.get("error").getAsString()
                        : "The Claude Code view could not carry out '" + action + "'.");
            }
            out.remove("ok");
            return McpToolResult.success(out);
        } catch (RuntimeException e) {
            return McpToolResult.error("The Claude Code view gave an unreadable answer to '" + action
                    + "': " + e.getClass().getSimpleName());
        }
    }

    /** What the history panel lists, for the folder in front or the one named. Answered here,
     *  not by the page. */
    private McpToolResult listHistory(JsonObject params) {
        String folder;
        int limit;
        try {
            folder = folderOf(params);
            limit = limitOf(params);
        } catch (IllegalArgumentException e) {
            return McpToolResult.error(e.getMessage());
        }
        String query = ClaudeCodeEclipseTool.str(params, "query");
        String wanted = query == null ? null : query.toLowerCase(Locale.ROOT);

        JsonArray all = sessions(folder);
        JsonArray shown = new JsonArray();
        int total = 0;
        for (JsonElement e : all) {
            JsonObject session = e.getAsJsonObject();
            String title = title(text(session, "display"));
            if (wanted != null && !title.toLowerCase(Locale.ROOT).contains(wanted)) continue;
            total++;
            if (shown.size() >= limit) continue;
            JsonObject one = new JsonObject();
            one.addProperty("sessionId", text(session, "sessionId"));
            one.addProperty("title", title);
            one.addProperty("lastActive", text(session, "timestamp"));
            shown.add(one);
        }
        JsonObject out = new JsonObject();
        out.addProperty("folder", folder);
        out.addProperty("total", total);
        out.add("sessions", shown);
        if (all.size() >= MOST_LISTED) {
            JsonArray notes = new JsonArray();
            notes.add("Only a folder's " + MOST_LISTED + " most recent conversations are listed. An older "
                    + "one still opens by its id.");
            out.add("notes", notes);
        }
        return McpToolResult.success(out);
    }

    /**
     * Checks an 'openSession' call and gives the page what it needs to carry it out: the folder
     * spelled out, and the conversation's title.
     *
     * @return why it cannot be opened, or null when it can
     */
    private String describeSession(JsonObject params, JsonObject request) {
        String id = ClaudeCodeEclipseTool.str(params, "sessionId");
        if (id == null) return "'sessionId' is required. Use action 'listHistory'.";
        if (!SESSION_ID.matcher(id).matches()) {
            return "'" + id + "' is not a conversation id. Use action 'listHistory'.";
        }
        String folder;
        try {
            folder = folderOf(params);
        } catch (IllegalArgumentException e) {
            return e.getMessage();
        }
        String title = null;
        for (JsonElement e : sessions(folder)) {
            JsonObject session = e.getAsJsonObject();
            if (id.equals(text(session, "sessionId"))) {
                title = title(text(session, "display"));
                break;
            }
        }
        // Being listed is proof enough. Its file is looked for only when it is not: the list
        // ends at a folder's most recent conversations, and an older one is still there.
        if (title == null && !history.has(folder, id)) {
            return "There is no saved conversation '" + id + "' in " + folder + ". Use action 'listHistory'.";
        }
        request.addProperty("sessionId", id);
        request.addProperty("folder", folder);
        // No title of its own leaves the tab its usual name rather than "(untitled)".
        request.addProperty("title", title == null || UNTITLED.equals(title) ? "" : title);
        return null;
    }

    /** A conversation's title as the history panel shows it. */
    static String title(String display) {
        String t = display == null ? "" : display;
        t = IDE_BLOCK.matcher(t).replaceAll("");
        t = IDE_EMPTY.matcher(t).replaceAll("");
        t = META_BLOCK.matcher(t).replaceAll("");
        t = COMMAND_NAME.matcher(t).replaceAll("$1");   // the command itself stays, e.g. /usage
        t = t.strip();
        return t.isEmpty() ? UNTITLED : t;
    }

    /** The folder a call is about: the one it names, as a full path, or the one in front. */
    private String folderOf(JsonObject params) {
        String named = ClaudeCodeEclipseTool.str(params, "folder");
        if (named == null) return history.frontFolder();
        try {
            Path path = Paths.get(named);
            // Anything else would be read against Eclipse's own working directory.
            if (path.isAbsolute()) return path.normalize().toString();
        } catch (InvalidPathException e) {
            // refused below
        }
        throw new IllegalArgumentException("'folder' must be the full path of a folder, such as a 'root' "
                + "that 'listTabs' shows.");
    }

    private static int limitOf(JsonObject params) {
        if (!params.has("limit") || params.get("limit").isJsonNull()) return LISTED;
        try {
            int limit = params.get("limit").getAsInt();
            if (limit >= 1 && limit <= MOST_LISTED) return limit;
        } catch (RuntimeException e) {
            // refused below
        }
        throw new IllegalArgumentException("'limit' must be a number from 1 to " + MOST_LISTED + ".");
    }

    /** A folder's conversations, as objects; none when the list cannot be read. */
    private JsonArray sessions(String folder) {
        JsonArray out = new JsonArray();
        try {
            for (JsonElement e : JsonParser.parseString(history.list(folder)).getAsJsonArray()) {
                if (e.isJsonObject()) out.add(e);
            }
        } catch (RuntimeException e) {
            // an unreadable list is an empty one
        }
        return out;
    }

    private static String text(JsonObject o, String key) {
        return o.has(key) && o.get(key).isJsonPrimitive() ? o.get(key).getAsString() : "";
    }

    private static String askView(String requestJson) {
        ClaudeCodeView.debug("[claudeCodeEclipse]claudeCodeView request " + requestJson);
        String reply = UiHelper.syncCall(() -> ClaudeGuiView.pageTool(requestJson));
        ClaudeCodeView.debug("[claudeCodeEclipse]claudeCodeView reply " + reply);
        return reply;
    }

    private static JsonObject param(String type, String description) {
        JsonObject p = new JsonObject();
        p.addProperty("type", type);
        p.addProperty("description", description);
        return p;
    }
}
