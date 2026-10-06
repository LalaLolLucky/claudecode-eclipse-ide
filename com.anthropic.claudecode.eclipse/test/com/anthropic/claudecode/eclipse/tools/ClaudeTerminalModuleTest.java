package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayList;
import java.util.List;

import org.junit.jupiter.api.Test;

import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.TerminalTab;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Tests for the Claude Terminal's part of the claudeCodeEclipse tool: opening a tab and
 * typing a slash command into one.
 */
class ClaudeTerminalModuleTest {

    /** A Claude Terminal view with the given tabs, recording what is typed into it. */
    private static final class Terminal implements ClaudeTerminalModule.Terminal {
        final List<TerminalTab> tabs = new ArrayList<>();
        final List<String> typed = new ArrayList<>();
        boolean opens = true;
        boolean takesInput = true;

        Terminal with(String id, boolean active, boolean started, boolean ended) {
            tabs.add(new TerminalTab(id, "Claude " + id, active, started, ended));
            return this;
        }

        @Override
        public List<TerminalTab> tabs() {
            return List.copyOf(tabs);
        }

        @Override
        public String open() {
            if (!opens) return null;
            String id = "term" + (tabs.size() + 1);
            for (int i = 0; i < tabs.size(); i++) {
                TerminalTab t = tabs.get(i);
                tabs.set(i, new TerminalTab(t.id(), t.title(), false, t.started(), t.ended()));
            }
            tabs.add(new TerminalTab(id, "Claude " + (tabs.size() + 1), true, false, false));
            return id;
        }

        @Override
        public boolean submit(String tabId, String text) {
            if (!takesInput) return false;
            typed.add(tabId + " <- " + text);
            return true;
        }
    }

    private static McpToolResult run(Terminal terminal, String action, String tabId, String command) {
        JsonObject call = new JsonObject();
        call.addProperty("module", "claudeTerminal");
        call.addProperty("action", action);
        if (tabId != null) call.addProperty("tabId", tabId);
        if (command != null) call.addProperty("command", command);
        return new ClaudeCodeEclipseTool(new ClaudeTerminalModule(terminal)).execute(call);
    }

    private static String text(McpToolResult result) {
        return result.content().get(0).getAsJsonObject().get("text").getAsString();
    }

    private static JsonObject json(McpToolResult result) {
        return JsonParser.parseString(text(result)).getAsJsonObject();
    }

    @Test
    void aNewTabIsOpenedAndComesBackWithTheOthers() {
        Terminal terminal = new Terminal().with("term1", true, true, false);

        McpToolResult result = run(terminal, "newTab", null, null);

        assertFalse(result.isError());
        JsonObject out = json(result);
        assertEquals("term2", out.getAsJsonObject("tab").get("id").getAsString());
        assertTrue(out.getAsJsonObject("tab").get("active").getAsBoolean());
        assertFalse(out.getAsJsonObject("tab").get("started").getAsBoolean());
        assertEquals(2, out.getAsJsonArray("tabs").size());
    }

    @Test
    void aTabTheViewDidNotOpenIsNotReportedAsOpened() {
        Terminal terminal = new Terminal().with("term1", true, true, false);
        terminal.opens = false;

        McpToolResult result = run(terminal, "newTab", null, null);

        assertTrue(result.isError());
        assertEquals(1, terminal.tabs.size());
    }

    @Test
    void aCommandIsTypedIntoTheTabItNames() {
        Terminal terminal = new Terminal().with("term1", true, true, false).with("term2", false, true, false);

        McpToolResult result = run(terminal, "runCommand", "term2", "  /review src/x  ");

        assertFalse(result.isError());
        assertEquals(List.of("term2 <- /review src/x"), terminal.typed);
        assertEquals("term2", json(result).getAsJsonObject("tab").get("id").getAsString());
        assertEquals(2, json(result).getAsJsonArray("tabs").size());
    }

    @Test
    void withNoTabNamedItGoesToTheTabInFront() {
        Terminal terminal = new Terminal().with("term1", false, true, false).with("term2", true, true, false);

        McpToolResult result = run(terminal, "runCommand", null, "/compact");

        assertFalse(result.isError());
        assertEquals(List.of("term2 <- /compact"), terminal.typed);
    }

    @Test
    void withNoTerminalOpenNothingIsTypedAndTheAnswerSaysHowToOpenOne() {
        Terminal terminal = new Terminal();

        McpToolResult result = run(terminal, "runCommand", null, "/compact");

        assertTrue(result.isError());
        assertTrue(text(result).contains("newTab"));
        assertTrue(terminal.typed.isEmpty());
    }

    @Test
    void aTabThatIsNotThereIsRefusedAndTheOnesThatAreAreNamed() {
        Terminal terminal = new Terminal().with("term1", true, true, false);

        McpToolResult result = run(terminal, "runCommand", "term9", "/compact");

        assertTrue(result.isError());
        assertTrue(text(result).contains("term1"));
        assertTrue(terminal.typed.isEmpty());
    }

    @Test
    void aTabStillStartingOrAlreadyEndedIsTypedNothing() {
        Terminal terminal = new Terminal().with("term1", true, false, false).with("term2", false, true, true);

        McpToolResult starting = run(terminal, "runCommand", "term1", "/compact");
        McpToolResult ended = run(terminal, "runCommand", "term2", "/compact");

        assertTrue(starting.isError());
        assertTrue(ended.isError());
        assertTrue(terminal.typed.isEmpty());
    }

    @Test
    void onlyOneLineThatIsASlashCommandIsTyped() {
        assertNull(ClaudeTerminalModule.commandError("/compact"));
        assertNull(ClaudeTerminalModule.commandError("/model opus"));
        assertNotNull(ClaudeTerminalModule.commandError(null));
        assertNotNull(ClaudeTerminalModule.commandError(""));
        assertNotNull(ClaudeTerminalModule.commandError("compact"));
        assertNotNull(ClaudeTerminalModule.commandError("/"));
        assertNotNull(ClaudeTerminalModule.commandError("/ compact"));
        // Enter would submit at the break and leave the rest to be read as a prompt.
        assertNotNull(ClaudeTerminalModule.commandError("/compact\nrm -rf x"));
        assertNotNull(ClaudeTerminalModule.commandError("/compact\rnow"));
        // Escape and the other control characters are keys to the terminal program.
        assertNotNull(ClaudeTerminalModule.commandError("/model\u001b[A"));
        assertNotNull(ClaudeTerminalModule.commandError("/model\topus"));
    }

    @Test
    void aCommandThatIsNotOneIsRefusedBeforeAnyTabIsLookedFor() {
        Terminal terminal = new Terminal().with("term1", true, true, false);

        McpToolResult result = run(terminal, "runCommand", "term1", "please compact");

        assertTrue(result.isError());
        assertTrue(terminal.typed.isEmpty());
    }

    @Test
    void aCommandTheTerminalDidNotTakeIsAnError() {
        Terminal terminal = new Terminal().with("term1", true, true, false);
        terminal.takesInput = false;

        McpToolResult result = run(terminal, "runCommand", "term1", "/compact");

        assertTrue(result.isError());
    }

    @Test
    void anActionItDoesNotHaveIsRefused() {
        Terminal terminal = new Terminal().with("term1", true, true, false);

        McpToolResult result = run(terminal, "closeTab", "term1", null);

        assertTrue(result.isError());
        assertTrue(text(result).contains("runCommand"));
    }
}
