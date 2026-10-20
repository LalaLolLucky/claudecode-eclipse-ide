package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/** Tests for the Claude IDE Server view's part of the claudeCodeEclipse tool: reading its log. */
class ClaudeIdeServerModuleTest {

    /** A Claude IDE Server view showing {@code log}. */
    private static ClaudeIdeServerModule over(String log, boolean open, boolean debugMode) {
        return new ClaudeIdeServerModule(new ClaudeIdeServerModule.Server() {
            @Override public String log() { return log; }
            @Override public boolean open() { return open; }
            @Override public boolean debugMode() { return debugMode; }
        });
    }

    private static JsonObject read(ClaudeIdeServerModule module, Integer lines) {
        JsonObject params = new JsonObject();
        if (lines != null) params.addProperty("lines", lines);
        McpToolResult result = module.run("readLog", params);
        assertFalse(result.isError(), text(result));
        return JsonParser.parseString(text(result)).getAsJsonObject();
    }

    private static String text(McpToolResult result) {
        return result.content().get(0).getAsJsonObject().get("text").getAsString();
    }

    @Test
    void theWholeLogComesBackWhenNoLinesAreAskedFor() {
        String log = "Claude Code for Eclipse IDE (Unofficial)\n\n[mcp] → list\n[cli] ← ok\n";
        JsonObject out = read(over(log, true, true), null);
        assertEquals(log, out.get("log").getAsString());
        assertTrue(out.get("viewOpen").getAsBoolean());
        assertTrue(out.get("debugMode").getAsBoolean());
    }

    @Test
    void linesGivesTheLastOnes() {
        JsonObject out = read(over("a\nb\nc\nd\n", true, true), 2);
        assertEquals("c\nd\n", out.get("log").getAsString());
    }

    @Test
    void moreLinesThanTheLogHasGivesItAll() {
        JsonObject out = read(over("a\nb\n", true, true), 50);
        assertEquals("a\nb\n", out.get("log").getAsString());
    }

    @Test
    void aClosedViewStillGivesWhatWasKeptForIt() {
        JsonObject out = read(over("[Bridge] Relay up\n", false, true), null);
        assertEquals("[Bridge] Relay up\n", out.get("log").getAsString());
        assertFalse(out.get("viewOpen").getAsBoolean());
    }

    @Test
    void withDebugModeOffAndNothingKeptItSaysSo() {
        JsonObject out = read(over("", false, false), null);
        assertEquals("", out.get("log").getAsString());
        assertFalse(out.get("debugMode").getAsBoolean());
        assertTrue(out.get("note").getAsString().contains("Debug mode"), out.toString());
    }

    @Test
    void anUnknownActionIsRefusedWithTheOnesThereAre() {
        McpToolResult result = over("", true, true).run("clearLog", new JsonObject());
        assertTrue(result.isError());
        assertTrue(text(result).contains("readLog"), text(result));
    }

    @Test
    void itDescribesItselfAndItsLinesParameter() {
        ClaudeIdeServerModule module = over("", true, true);
        assertTrue(module.description().contains("readLog"));
        JsonObject props = new JsonObject();
        module.describeParams(props);
        assertTrue(props.getAsJsonObject("lines").get("description").getAsString().contains("claudeIdeServer readLog"));
    }
}
