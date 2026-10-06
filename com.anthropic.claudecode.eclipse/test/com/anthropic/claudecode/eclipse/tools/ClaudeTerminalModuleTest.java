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
 * Tests for the Claude Terminal's part of the claudeCodeEclipse tool: opening and closing a
 * tab, typing a prompt or a slash command into one, and what can be set in one by a command.
 */
class ClaudeTerminalModuleTest {

    /** A Claude Terminal view with the given tabs, recording what is done to it. */
    private static final class Terminal implements ClaudeTerminalModule.Terminal {
        final List<TerminalTab> tabs = new ArrayList<>();
        final List<String> typed = new ArrayList<>();
        final List<String> switched = new ArrayList<>();
        final List<String> closed = new ArrayList<>();
        boolean opens = true;
        boolean takesInput = true;

        Terminal with(String id, boolean active, boolean started, boolean ended) {
            return with(id, active, started, ended, "off");
        }

        Terminal with(String id, boolean active, boolean started, boolean ended, String remoteControl) {
            tabs.add(new TerminalTab(id, "Claude " + id, active, started, ended, remoteControl));
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
                tabs.set(i, new TerminalTab(t.id(), t.title(), false, t.started(), t.ended(), t.remoteControl()));
            }
            tabs.add(new TerminalTab(id, "Claude " + (tabs.size() + 1), true, false, false, "off"));
            return id;
        }

        @Override
        public boolean submit(String tabId, String text) {
            if (!takesInput) return false;
            typed.add(tabId + " <- " + text);
            return true;
        }

        @Override
        public boolean remoteControl(String tabId, boolean on) {
            if (!takesInput) return false;
            switched.add(tabId + (on ? " on" : " off"));
            for (int i = 0; i < tabs.size(); i++) {
                TerminalTab t = tabs.get(i);
                if (t.id().equals(tabId)) {
                    tabs.set(i, new TerminalTab(t.id(), t.title(), t.active(), t.started(), t.ended(),
                            on ? "on" : "disconnecting"));
                }
            }
            return true;
        }

        @Override
        public boolean close(String tabId) {
            closed.add(tabId);
            return tabs.removeIf(t -> t.id().equals(tabId));
        }
    }

    private static JsonObject call(String action, String tabId) {
        JsonObject call = new JsonObject();
        call.addProperty("module", "claudeTerminal");
        call.addProperty("action", action);
        if (tabId != null) call.addProperty("tabId", tabId);
        return call;
    }

    private static McpToolResult run(Terminal terminal, JsonObject call) {
        return new ClaudeCodeEclipseTool(new ClaudeTerminalModule(terminal)).execute(call);
    }

    private static McpToolResult run(Terminal terminal, String action, String tabId, String command) {
        JsonObject call = call(action, tabId);
        if (command != null) call.addProperty("command", command);
        return run(terminal, call);
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
        assertEquals("off", out.getAsJsonObject("tab").get("remoteControl").getAsString());
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

        McpToolResult result = run(terminal, "listTabs", "term1", null);

        assertTrue(result.isError());
        assertTrue(text(result).contains("runCommand"));
    }

    // ── sendPrompt ──

    @Test
    void aPromptIsTypedIntoTheTabItNamesOrTheOneInFront() {
        Terminal terminal = new Terminal().with("term1", true, true, false).with("term2", false, true, false);
        JsonObject named = call("sendPrompt", "term2");
        named.addProperty("prompt", "  run the tests  ");
        JsonObject front = call("sendPrompt", null);
        front.addProperty("prompt", "and the linter");

        assertFalse(run(terminal, named).isError());
        assertFalse(run(terminal, front).isError());

        assertEquals(List.of("term2 <- run the tests", "term1 <- and the linter"), terminal.typed);
    }

    @Test
    void aPromptMustBeOneLineOfPlainTextAndNotACommand() {
        assertNull(ClaudeTerminalModule.promptError("run the tests"));
        assertNotNull(ClaudeTerminalModule.promptError(null));
        assertNotNull(ClaudeTerminalModule.promptError("   "));
        // In a terminal the break submits what is typed so far, and the rest follows as another message.
        assertNotNull(ClaudeTerminalModule.promptError("first line\nsecond line"));
        assertNotNull(ClaudeTerminalModule.promptError("press\u001bescape"));
        assertTrue(ClaudeTerminalModule.promptError("/compact").contains("runCommand"));
    }

    @Test
    void aPromptThatCannotBeTypedLeavesTheTerminalAlone() {
        Terminal terminal = new Terminal().with("term1", true, true, false).with("term2", false, false, false);
        JsonObject twoLines = call("sendPrompt", "term1");
        twoLines.addProperty("prompt", "first\nsecond");
        JsonObject starting = call("sendPrompt", "term2");
        starting.addProperty("prompt", "hello");

        assertTrue(run(terminal, twoLines).isError());
        assertTrue(run(terminal, starting).isError());
        assertTrue(run(terminal, call("sendPrompt", "term1")).isError());
        assertTrue(terminal.typed.isEmpty());
    }

    // ── closeTab ──

    @Test
    void aTabIsClosedByItsIdAndTheRestComeBack() {
        Terminal terminal = new Terminal().with("term1", true, true, false).with("term2", false, true, true);

        McpToolResult result = run(terminal, call("closeTab", "term2"));

        assertFalse(result.isError());
        assertEquals(List.of("term2"), terminal.closed);
        assertEquals("term2", json(result).get("closed").getAsString());
        assertEquals(1, json(result).getAsJsonArray("tabs").size());
    }

    @Test
    void closingNeedsATabThatIsThere() {
        Terminal terminal = new Terminal().with("term1", true, true, false);

        assertTrue(run(terminal, call("closeTab", null)).isError());
        assertTrue(run(terminal, call("closeTab", "term9")).isError());
        assertTrue(terminal.closed.isEmpty());
    }

    // ── configureTab ──

    @Test
    void theModelAndTheEffortAreSetByTypingTheirCommandsInThatOrder() {
        Terminal terminal = new Terminal().with("term1", true, true, false);
        JsonObject both = call("configureTab", "term1");
        both.addProperty("effort", "XHIGH");
        both.addProperty("model", "sonnet[1m]");

        McpToolResult result = run(terminal, both);

        assertFalse(result.isError());
        assertEquals(List.of("term1 <- /model sonnet[1m]", "term1 <- /effort xhigh"), terminal.typed);
    }

    @Test
    void oneOfTheTwoAloneTypesOnlyItsOwnCommand() {
        Terminal terminal = new Terminal().with("term1", true, true, false);
        JsonObject model = call("configureTab", null);
        model.addProperty("model", "opus");
        JsonObject effort = call("configureTab", null);
        effort.addProperty("effort", "low");

        run(terminal, model);
        run(terminal, effort);

        assertEquals(List.of("term1 <- /model opus", "term1 <- /effort low"), terminal.typed);
    }

    @Test
    void autoIsAnEffortTheTerminalTakesToo() {
        Terminal terminal = new Terminal().with("term1", true, true, false);
        JsonObject auto = call("configureTab", "term1");
        auto.addProperty("effort", "Auto");

        McpToolResult result = run(terminal, auto);

        assertFalse(result.isError());
        assertEquals(List.of("term1 <- /effort auto"), terminal.typed);
    }

    @Test
    void thinkingAndTheModeCannotBeSetInATerminalAndNothingIsTyped() {
        Terminal terminal = new Terminal().with("term1", true, true, false);
        JsonObject thinking = call("configureTab", "term1");
        thinking.addProperty("model", "opus");
        thinking.addProperty("thinking", true);
        JsonObject mode = call("configureTab", "term1");
        mode.addProperty("mode", "plan");

        McpToolResult a = run(terminal, thinking);
        McpToolResult b = run(terminal, mode);

        assertTrue(a.isError());
        assertTrue(b.isError());
        assertTrue(text(a).contains("model") && text(a).contains("effort"), text(a));
        assertTrue(terminal.typed.isEmpty());
    }

    @Test
    void aSettingThatIsNotOneIsRefusedBeforeAnythingIsTyped() {
        Terminal terminal = new Terminal().with("term1", true, true, false);
        JsonObject nothing = call("configureTab", "term1");
        JsonObject effort = call("configureTab", "term1");
        effort.addProperty("model", "opus");
        effort.addProperty("effort", "turbo");
        JsonObject model = call("configureTab", "term1");
        model.addProperty("model", "opus; rm -rf x");

        assertTrue(run(terminal, nothing).isError());
        assertTrue(run(terminal, effort).isError());
        assertTrue(run(terminal, model).isError());
        assertTrue(terminal.typed.isEmpty());
    }

    // ── remoteControl ──

    private static JsonObject remote(String tabId, boolean enabled) {
        JsonObject call = call("remoteControl", tabId);
        call.addProperty("enabled", enabled);
        return call;
    }

    @Test
    void remoteControlIsSwitchedOnlyWhenItIsNotAlreadyAsAsked() {
        Terminal terminal = new Terminal().with("term1", true, true, false, "off")
                .with("term2", false, true, false, "on");

        McpToolResult on = run(terminal, remote("term1", true));
        McpToolResult alreadyOn = run(terminal, remote("term2", true));
        McpToolResult alreadyOff = run(terminal, remote("term1", false));

        assertFalse(on.isError());
        assertFalse(alreadyOn.isError());
        // term1 is on by now, so switching it off is a switch.
        assertFalse(alreadyOff.isError());
        assertEquals(List.of("term1 on", "term1 off"), terminal.switched);
        assertEquals("on", json(on).getAsJsonObject("tab").get("remoteControl").getAsString());
        assertEquals("disconnecting", json(alreadyOff).getAsJsonObject("tab").get("remoteControl").getAsString());
    }

    @Test
    void switchingOffATabThatIsOffTypesNothing() {
        Terminal terminal = new Terminal().with("term1", true, true, false, "off");

        McpToolResult result = run(terminal, remote(null, false));

        assertFalse(result.isError());
        assertTrue(terminal.switched.isEmpty());
        assertEquals("off", json(result).getAsJsonObject("tab").get("remoteControl").getAsString());
    }

    @Test
    void aTabOnItsWayOffTakesNothingUntilItIsThere() {
        Terminal terminal = new Terminal().with("term1", true, true, false, "disconnecting");
        JsonObject prompt = call("sendPrompt", "term1");
        prompt.addProperty("prompt", "hello");

        assertTrue(run(terminal, remote("term1", true)).isError());
        assertTrue(run(terminal, "runCommand", "term1", "/compact").isError());
        assertTrue(run(terminal, prompt).isError());
        assertTrue(terminal.switched.isEmpty());
        assertTrue(terminal.typed.isEmpty());
    }

    @Test
    void enabledHasToBeTrueOrFalse() {
        Terminal terminal = new Terminal().with("term1", true, true, false);
        JsonObject word = call("remoteControl", "term1");
        word.addProperty("enabled", "on");

        assertTrue(run(terminal, call("remoteControl", "term1")).isError());
        assertTrue(run(terminal, word).isError());
        assertTrue(terminal.switched.isEmpty());
    }
}
