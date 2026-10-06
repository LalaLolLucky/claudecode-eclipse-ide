package com.anthropic.claudecode.eclipse.ui;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Predicate;

import com.google.gson.JsonObject;

/**
 * What the Claude Terminal adds to the {@code claude} command line, and to the settings file
 * it passes, for the preferences that reach it: the plug-in's tools, bypass permissions mode,
 * Remote Control on startup and thinking by default.
 *
 * <p>Kept apart from {@link ClaudeCliView} because every line of it is a decision that can be
 * wrong for somebody's installed CLI: a flag an older CLI does not know makes it exit at once
 * ("unknown option") and the terminal with it. So nothing here is passed on the strength of
 * the preference alone — the caller says which flags the CLI knows, and a flag it does not
 * know is left out, which is the terminal exactly as it was before these preferences.
 */
final class TerminalLaunchArgs {

    static final String MCP_CONFIG = "--mcp-config";
    static final String STRICT_MCP_CONFIG = "--strict-mcp-config";
    static final String DISALLOWED_TOOLS = "--disallowed-tools";
    /** The deny list's older spelling, the only one early CLIs take. */
    static final String DISALLOWED_TOOLS_OLD = "--disallowedTools";
    static final String ALLOW_BYPASS = "--allow-dangerously-skip-permissions";
    static final String BYPASS = "--dangerously-skip-permissions";
    static final String REMOTE_CONTROL = "--remote-control";
    static final String REMOTE_CONTROL_SHORT = "--rc";

    /** Every flag {@link #build} may pass: what the caller has to ask the CLI about. */
    static final List<String> PROBED = List.of(
            MCP_CONFIG, DISALLOWED_TOOLS, DISALLOWED_TOOLS_OLD, ALLOW_BYPASS, REMOTE_CONTROL);

    /** The name the plug-in's server goes by, as in the Claude Code view's own launch. */
    private static final String SERVER = "eclipse";

    /**
     * Tools of the plug-in's server the Terminal's model is not shown.
     *
     * <p>The diff tools belong to the CLI: it opens the diff editor itself, over the IDE
     * link, when an edit needs review, and a model holding {@code acceptDiff} could approve
     * its own edit. The other two raise the Claude Code view's cards, which the Terminal
     * has its own prompts for. The IDE link's {@code mcp__ide__openDiff} is deliberately
     * not here — denying it is how the Claude Code view switches the diff editor off.
     */
    private static final List<String> HIDDEN_TOOLS = List.of(
            "openDiff", "acceptDiff", "rejectDiff", "closeAllDiffTabs", "askUserQuestion", "approvalPrompt");

    /** The preferences that add something to the command line. */
    record Options(boolean mcpTools, boolean bypass, boolean remoteControl) {
    }

    private TerminalLaunchArgs() {
    }

    /**
     * The arguments to append after the user's own.
     *
     * @param userArgs      what the launch already carries: the Arguments preference and the
     *                      launch's own (a resume id), so nothing of the user's is repeated
     * @param cliKnows      whether the installed CLI knows a flag of {@link #PROBED}
     * @param mcpConfigFile the file naming the plug-in's server, null when it could not be
     *                      written
     */
    static List<String> build(List<String> userArgs, Options options, Predicate<String> cliKnows,
            String mcpConfigFile) {
        List<String> args = new ArrayList<>();

        // Lets the session be switched INTO bypass mode; it does not start in it.
        if (options.bypass() && !has(userArgs, ALLOW_BYPASS) && !has(userArgs, BYPASS)
                && cliKnows.test(ALLOW_BYPASS)) {
            args.add(ALLOW_BYPASS);
        }

        // Both flags take several values and both add to the user's own (verified against
        // CLI 2.1.287), so a server or a deny list in the Arguments preference stays in
        // force. --strict-mcp-config is the user saying "only mine", and is taken at that.
        if (options.mcpTools() && mcpConfigFile != null && !has(userArgs, STRICT_MCP_CONFIG)
                && cliKnows.test(MCP_CONFIG)) {
            String deny = cliKnows.test(DISALLOWED_TOOLS) ? DISALLOWED_TOOLS
                    : cliKnows.test(DISALLOWED_TOOLS_OLD) ? DISALLOWED_TOOLS_OLD : null;
            // No way to hide the diff tools means no server: see HIDDEN_TOOLS.
            if (deny != null) {
                args.add(MCP_CONFIG);
                args.add(mcpConfigFile);
                args.add(deny);
                args.add(hiddenTools());
            }
        }

        // Last: its name is optional, so whatever follows it must be another flag or
        // nothing, and the caller only ever appends flags after these.
        if (options.remoteControl() && !has(userArgs, REMOTE_CONTROL) && !has(userArgs, REMOTE_CONTROL_SHORT)
                && cliKnows.test(REMOTE_CONTROL)) {
            args.add(REMOTE_CONTROL);
        }
        return args;
    }

    /**
     * Thinking by default, in the settings file the Terminal passes. Left on, nothing is
     * written: a file passed at launch outranks the user's own settings, and the CLI's
     * default is already on. Only switching it off says anything.
     */
    static void applyThinking(JsonObject settings, boolean thinkingByDefault) {
        if (!thinkingByDefault) settings.addProperty("alwaysThinkingEnabled", false);
    }

    /** The {@code --mcp-config} file's contents: the plug-in's server on {@code port}. */
    static String mcpConfigJson(int port) {
        return "{\"mcpServers\":{\"" + SERVER + "\":{\"type\":\"sse\",\"url\":\"http://127.0.0.1:" + port
                + "/sse\"}}}";
    }

    private static String hiddenTools() {
        List<String> qualified = new ArrayList<>();
        for (String tool : HIDDEN_TOOLS) qualified.add("mcp__" + SERVER + "__" + tool);
        return String.join(",", qualified);
    }

    /** Whether the user's arguments carry {@code flag}, with or without {@code =value}. */
    private static boolean has(List<String> userArgs, String flag) {
        for (String arg : userArgs) {
            if (arg.equals(flag) || arg.startsWith(flag + "=")) return true;
        }
        return false;
    }
}
