package com.anthropic.claudecode.eclipse.tools;

import java.util.List;
import java.util.function.BooleanSupplier;
import java.util.function.Function;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.anthropic.claudecode.eclipse.ui.ClaudeGuiView;
import com.anthropic.claudecode.eclipse.ui.TerminalOnlyUi;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * The Claude Code view's part of {@link ClaudeCodeEclipseTool}: its conversation tabs — opening
 * and closing them, sending a prompt into one, switching Remote Control on or off for one — and
 * each tab's model, effort, thinking and permission mode.
 *
 * <p>The tabs and their settings live in the view's page, so every action is carried out
 * there ({@code scripts/claudetool.js}) by the same code paths the composer's own controls
 * use; this class only checks the action, passes its parameters across and turns the page's
 * reply into a tool result.
 */
class ClaudeCodeViewModule implements ClaudeCodeEclipseTool.Module {

    private static final List<String> ACTIONS = List.of("listTabs", "newTab", "configureTab", "sendPrompt", "remoteControl", "closeTab");

    /** What an action may send to the page; anything else in the call stays behind. */
    private static final List<String> PARAMS = List.of("tabId", "folder", "model", "effort", "thinking", "mode", "prompt", "enabled", "remoteControl");

    /** Asks the page: request JSON in, reply JSON out, null when there is no page to ask. */
    private final Function<String, String> page;

    /** Whether the view is switched off altogether ("Exclusively use terminal"). */
    private final BooleanSupplier viewOff;

    ClaudeCodeViewModule() {
        this(ClaudeCodeViewModule::askView, TerminalOnlyUi::isOn);
    }

    ClaudeCodeViewModule(Function<String, String> page) {
        this(page, () -> false);
    }

    ClaudeCodeViewModule(Function<String, String> page, BooleanSupplier viewOff) {
        this.page = page;
        this.viewOff = viewOff;
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
                + "models, efforts and modes a tab can be given. action='newTab' opens a new "
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
                + "action='remoteControl' switches Remote Control on or off ('enabled') for the tab "
                + "with 'tabId'; it connects in the background, so 'listTabs' shows 'connecting' "
                + "and then 'on', and switching it off for the tab the user is talking through "
                + "from another device cuts them off from it. "
                + "action='closeTab' closes the tab with 'tabId', stopping whatever it is doing. "
                + "A call cannot tell which tab it was made from, so take 'tabId' from 'listTabs' "
                + "or 'newTab'; closing the tab the call came from ends this conversation.";
    }

    @Override
    public void describeParams(JsonObject props) {
        props.add("tabId", param("string", "claudeCodeView configureTab/sendPrompt/remoteControl/closeTab: "
                + "the tab, as shown by 'listTabs', e.g. 'tab2'. Required for all but 'configureTab'; "
                + "'configureTab' without it changes the tab in front."));
        props.add("folder", param("string", "claudeCodeView newTab: the full path of the folder the "
                + "conversation runs in, such as a 'root' from 'listTabs'. Left out, the folder tab in front."));
        props.add("prompt", param("string", "claudeCodeView sendPrompt/newTab: the message to send. "
                + "Slash commands are not accepted."));
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
        JsonObject request = new JsonObject();
        request.addProperty("action", action);
        for (String key : PARAMS) {
            if (params.has(key) && !params.get(key).isJsonNull()) request.add(key, params.get(key));
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
