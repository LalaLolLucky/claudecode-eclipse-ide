package com.anthropic.claudecode.eclipse.tools;

import java.util.LinkedHashMap;
import java.util.Map;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Drives this plug-in's own views. One tool with a module per view, so a view gains its
 * actions without adding to the tool list: {@code module} picks the view and {@code action}
 * what to do in it. The Claude Code view ({@link ClaudeCodeViewModule}) and the Claude
 * Terminal ({@link ClaudeTerminalModule}) each have one.
 */
public class ClaudeCodeEclipseTool implements McpTool {

    /** One view's actions. */
    interface Module {

        /** The value of {@code module} that selects it. */
        String name();

        /** What it does, action by action, for the tool description. */
        String description();

        /** Its parameters other than {@code module} and {@code action}, added to the schema. */
        void describeParams(JsonObject props);

        McpToolResult run(String action, JsonObject params);
    }

    private final Map<String, Module> modules = new LinkedHashMap<>();

    public ClaudeCodeEclipseTool() {
        this(new ClaudeCodeViewModule(), new ClaudeTerminalModule());
    }

    ClaudeCodeEclipseTool(Module... modules) {
        for (Module module : modules) this.modules.put(module.name(), module);
    }

    @Override
    public String toolName() {
        return "claudeCodeEclipse";
    }

    @Override
    public String description() {
        StringBuilder text = new StringBuilder("Control the Claude Code plug-in's own views in this "
                + "Eclipse. 'module' picks the view and 'action' what to do in it.");
        for (Module module : modules.values()) {
            text.append(" module='").append(module.name()).append("': ").append(module.description());
        }
        return text.toString();
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");

        JsonObject props = new JsonObject();

        JsonObject module = new JsonObject();
        module.addProperty("type", "string");
        module.addProperty("description", "The view to act on.");
        JsonArray names = new JsonArray();
        for (String name : modules.keySet()) names.add(name);
        module.add("enum", names);
        props.add("module", module);

        JsonObject action = new JsonObject();
        action.addProperty("type", "string");
        action.addProperty("description", "What to do in that view; the actions are listed per module "
                + "in the tool description.");
        props.add("action", action);

        for (Module m : modules.values()) m.describeParams(props);

        schema.add("properties", props);
        JsonArray required = new JsonArray();
        required.add("module");
        required.add("action");
        schema.add("required", required);
        return schema;
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        String name = str(params, "module");
        Module module = name == null ? null : modules.get(name);
        if (module == null) {
            return McpToolResult.error((name == null ? "'module' is required." : "Unknown module: " + name + ".")
                    + " Use one of: " + String.join(", ", modules.keySet()) + ".");
        }
        String action = str(params, "action");
        if (action == null) return McpToolResult.error("'action' is required. " + module.description());
        return module.run(action, params);
    }

    /**
     * Adds a parameter to the schema. When a module before this one has a parameter of the
     * same name, this one's use of it is added to that description instead: the schema has
     * one entry per name, whichever modules read it.
     */
    static void param(JsonObject props, String name, String type, String description) {
        if (props.has(name)) {
            JsonObject shared = props.getAsJsonObject(name);
            shared.addProperty("description", shared.get("description").getAsString() + " " + description);
            return;
        }
        JsonObject p = new JsonObject();
        p.addProperty("type", type);
        p.addProperty("description", description);
        props.add(name, p);
    }

    static String str(JsonObject params, String key) {
        if (!params.has(key) || params.get(key).isJsonNull()) return null;
        String v = params.get(key).getAsString().trim();
        return v.isEmpty() ? null : v;
    }
}
