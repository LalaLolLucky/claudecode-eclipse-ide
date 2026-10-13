package com.anthropic.claudecode.eclipse.tools;

import java.util.function.Consumer;
import java.util.function.Function;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * The whole build procedure in one call: refresh, clean, build, refresh, build. Both builds
 * are full builds. The second refresh and build are there for what the first build wrote to
 * disk — generated sources, copied resources — which Eclipse has not seen until it is
 * refreshed again.
 *
 * <p>A front for {@link BuildTool}, run twice, in the way {@link CleanTool} is a front for
 * it run once: the scope, the refresh and the markers have one implementation, so "project
 * X" means the same thing to every build tool. {@code refresh}, {@code clean} and
 * {@code build} stay as they are for a single step.
 *
 * <p>With the workspace's automatic build on, Eclipse builds in the background as well. Forge
 * runs all the same and says so in its result.
 */
public class ForgeTool implements McpTool {

    static final String AUTO_BUILD_WARNING = "Build Automatically is on in this workspace. Forge ran "
            + "all five steps, but each refresh and the clean also start Eclipse's own background "
            + "build, so the same work may be done more than once and take longer, and a background "
            + "build may still have been running when the errors here were read, which can leave "
            + "them incomplete. Switch it off with the 'autoBuild' tool to have Forge do all the "
            + "building itself.";

    private final Function<JsonObject, McpToolResult> build;

    /** Where the debug lines go: the plug-in's debug log, which needs a running Eclipse. */
    private final Consumer<String> debug;

    public ForgeTool() {
        this(new BuildTool()::execute, ClaudeCodeView::debug);
    }

    ForgeTool(Function<JsonObject, McpToolResult> build, Consumer<String> debug) {
        this.build = build;
        this.debug = debug;
    }

    @Override
    public String toolName() {
        return "forge";
    }

    @Override
    public String description() {
        return "Run the whole build procedure in one call: refresh, clean, build, refresh, build. "
                + "Both builds are full builds; the second refresh and build pick up what the first "
                + "build generated. Scope: the named projects, or the whole workspace when none are "
                + "given. Returns the error and warning counts after each pass and the errors that "
                + "remain after the second. For a single step use 'refresh', 'clean' or 'build'. "
                + "When the result has a 'warning', pass it on to the user.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject props = new JsonObject();
        props.add("projects", ToolSupport.strings(
                "Project names to forge. Omit or leave empty for the whole workspace."));
        return ToolSupport.schema(props);   // no args means "forge everything"
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        debug.accept("[forge] " + params);
        try {
            McpToolResult first = build.apply(pass(params, "clean-rebuild"));
            if (first.isError()) return first;   // nothing ran: an unknown or closed project
            JsonObject one = read(first);

            McpToolResult second = build.apply(pass(params, "rebuild"));
            if (second.isError()) {
                return McpToolResult.error("Forge stopped in its second pass, after the first refresh, "
                        + "clean and build had run. " + text(second));
            }
            JsonObject two = read(second);

            JsonObject result = new JsonObject();
            JsonArray steps = new JsonArray();
            for (String step : new String[] { "refresh", "clean", "build", "refresh", "build" }) steps.add(step);
            result.add("steps", steps);
            copy(two, result, "scope");
            result.addProperty("elapsedMs", number(one, "elapsedMs") + number(two, "elapsedMs"));

            // Read from the first pass: it is what the setting was while Forge ran.
            boolean autoBuild = one.has("autoBuildEnabled") && one.get("autoBuildEnabled").getAsBoolean();
            result.addProperty("autoBuildEnabled", autoBuild);
            if (autoBuild) result.addProperty("warning", AUTO_BUILD_WARNING);

            result.add("firstPass", counts(one));
            result.add("secondPass", counts(two));
            for (String key : new String[] { "errorCount", "warningCount", "projects", "errors", "truncated", "note" }) {
                copy(two, result, key);
            }
            return McpToolResult.success(result);
        } catch (RuntimeException e) {
            debug.accept("[forge] failed: " + e);
            return McpToolResult.error("Forge failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }

    /** One pass of {@code build}: always refreshed first, on the projects Forge was given. */
    private static JsonObject pass(JsonObject params, String mode) {
        JsonObject args = new JsonObject();
        args.addProperty("mode", mode);
        args.addProperty("refresh", true);
        if (params != null && params.has("projects") && params.get("projects").isJsonArray()) {
            args.add("projects", params.getAsJsonArray("projects"));
        }
        return args;
    }

    private static JsonObject counts(JsonObject pass) {
        JsonObject c = new JsonObject();
        for (String key : new String[] { "elapsedMs", "errorCount", "warningCount" }) copy(pass, c, key);
        return c;
    }

    private static void copy(JsonObject from, JsonObject to, String key) {
        if (from.has(key)) to.add(key, from.get(key));
    }

    private static long number(JsonObject o, String key) {
        return o.has(key) ? o.get(key).getAsLong() : 0;
    }

    private static JsonObject read(McpToolResult result) {
        return JsonParser.parseString(text(result)).getAsJsonObject();
    }

    private static String text(McpToolResult result) {
        return result.content().get(0).getAsJsonObject().get("text").getAsString();
    }
}
