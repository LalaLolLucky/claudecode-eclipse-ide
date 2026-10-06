package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

import org.junit.jupiter.api.Test;

import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Tests for how {@link ClaudeCodeEclipseTool} routes a call to a module and how
 * {@link ClaudeCodeViewModule} talks to the page, with the page replaced by a function so no
 * view or display is needed.
 */
class ClaudeCodeEclipseToolTest {

    /** A page that records what it was asked and answers with a fixed reply. */
    private static final class Page {
        final List<JsonObject> asked = new ArrayList<>();
        final String reply;

        Page(String reply) {
            this.reply = reply;
        }

        String call(String requestJson) {
            asked.add(JsonParser.parseString(requestJson).getAsJsonObject());
            return reply;
        }
    }

    private static ClaudeCodeEclipseTool toolOver(Page page) {
        return new ClaudeCodeEclipseTool(new ClaudeCodeViewModule(page::call));
    }

    /** A full path on whichever system the tests run on, spelled as Java spells it. */
    private static final String FRONT = Paths.get("front").toAbsolutePath().normalize().toString();
    private static final String OTHER = Paths.get("other").toAbsolutePath().normalize().toString();

    /** Saved conversations: two in the folder in front, none anywhere else, recording what was read. */
    private static final class Saved implements ClaudeCodeViewModule.History {
        final List<String> read = new ArrayList<>();
        /** Whether a conversation's file is where Java looks for it. */
        boolean filesFound = true;
        String listed = "[{\"sessionId\":\"bbbb-2\",\"display\":\"Second one\",\"timestamp\":\"2026-10-06T10:00:00Z\"},"
                + "{\"sessionId\":\"aaaa-1\",\"display\":\"First one\",\"timestamp\":\"2026-10-05T09:00:00Z\"}]";

        @Override
        public String frontFolder() {
            return FRONT;
        }

        @Override
        public String list(String folder) {
            read.add(folder);
            return FRONT.equals(folder) ? listed : "[]";
        }

        @Override
        public boolean has(String folder, String sessionId) {
            // "old-0" is a conversation too old to be listed any more, but still on disk.
            return filesFound && FRONT.equals(folder) && List.of("aaaa-1", "bbbb-2", "old-0").contains(sessionId);
        }
    }

    private static ClaudeCodeEclipseTool toolOver(Page page, Saved saved) {
        return new ClaudeCodeEclipseTool(new ClaudeCodeViewModule(page::call, () -> false, saved));
    }

    private static JsonObject args(String module, String action) {
        JsonObject j = new JsonObject();
        if (module != null) j.addProperty("module", module);
        if (action != null) j.addProperty("action", action);
        return j;
    }

    private static String text(McpToolResult result) {
        return result.content().get(0).getAsJsonObject().get("text").getAsString();
    }

    @Test
    void theSchemaOffersTheModulesItHas() {
        JsonObject schema = new ClaudeCodeEclipseTool().inputSchema();
        JsonArray modules = schema.getAsJsonObject("properties").getAsJsonObject("module").getAsJsonArray("enum");

        assertEquals(2, modules.size());
        assertEquals("claudeCodeView", modules.get(0).getAsString());
        assertEquals("claudeTerminal", modules.get(1).getAsString());
    }

    @Test
    void aParameterTwoModulesShareIsDescribedForBoth() {
        JsonObject props = new ClaudeCodeEclipseTool().inputSchema().getAsJsonObject("properties");

        String tabId = props.getAsJsonObject("tabId").get("description").getAsString();
        String command = props.getAsJsonObject("command").get("description").getAsString();
        assertTrue(tabId.contains("claudeCodeView") && tabId.contains("claudeTerminal"), tabId);
        assertTrue(command.contains("claudeCodeView") && command.contains("claudeTerminal"), command);
    }

    @Test
    void theHistoryOfTheFolderInFrontIsListedNewestFirstWithoutAskingThePage() {
        Page page = new Page("{\"ok\":true}");
        Saved saved = new Saved();

        McpToolResult result = toolOver(page, saved).execute(args("claudeCodeView", "listHistory"));

        assertFalse(result.isError());
        JsonObject out = JsonParser.parseString(text(result)).getAsJsonObject();
        assertEquals(FRONT, out.get("folder").getAsString());
        assertEquals(2, out.get("total").getAsInt());
        JsonObject first = out.getAsJsonArray("sessions").get(0).getAsJsonObject();
        assertEquals("bbbb-2", first.get("sessionId").getAsString());
        assertEquals("Second one", first.get("title").getAsString());
        assertEquals("2026-10-06T10:00:00Z", first.get("lastActive").getAsString());
        assertTrue(page.asked.isEmpty());
    }

    @Test
    void aLimitAndATitleNarrowTheList() {
        Saved saved = new Saved();
        JsonObject one = args("claudeCodeView", "listHistory");
        one.addProperty("limit", 1);
        JsonObject titled = args("claudeCodeView", "listHistory");
        titled.addProperty("query", "FIRST");

        JsonObject limited = JsonParser.parseString(text(toolOver(new Page("{}"), saved).execute(one))).getAsJsonObject();
        JsonObject found = JsonParser.parseString(text(toolOver(new Page("{}"), saved).execute(titled))).getAsJsonObject();

        assertEquals(1, limited.getAsJsonArray("sessions").size());
        assertEquals(2, limited.get("total").getAsInt());
        assertEquals(1, found.getAsJsonArray("sessions").size());
        assertEquals(1, found.get("total").getAsInt());
        assertEquals("aaaa-1", found.getAsJsonArray("sessions").get(0).getAsJsonObject().get("sessionId").getAsString());
    }

    @Test
    void theHistoryOfAFolderThatIsNamedIsReadFromThatFolder() {
        Saved saved = new Saved();
        JsonObject call = args("claudeCodeView", "listHistory");
        call.addProperty("folder", OTHER);

        McpToolResult result = toolOver(new Page("{}"), saved).execute(call);

        assertFalse(result.isError());
        assertEquals(List.of(OTHER), saved.read);
        assertEquals(0, JsonParser.parseString(text(result)).getAsJsonObject().get("total").getAsInt());
    }

    @Test
    void aFolderThatIsNotAFullPathIsRefusedBeforeAnythingIsRead() {
        Saved saved = new Saved();
        JsonObject list = args("claudeCodeView", "listHistory");
        list.addProperty("folder", "work/api");
        JsonObject open = args("claudeCodeView", "openSession");
        open.addProperty("sessionId", "aaaa-1");
        open.addProperty("folder", "work/api");
        Page page = new Page("{\"ok\":true}");

        assertTrue(toolOver(page, saved).execute(list).isError());
        assertTrue(toolOver(page, saved).execute(open).isError());
        assertTrue(saved.read.isEmpty());
        assertTrue(page.asked.isEmpty());
    }

    @Test
    void aConversationIsOpenedByAskingThePageWithItsTitleAndFolder() {
        Page page = new Page("{\"ok\":true,\"tab\":{\"id\":\"tab5\"}}");
        JsonObject call = args("claudeCodeView", "openSession");
        call.addProperty("sessionId", "aaaa-1");
        call.addProperty("title", "not the caller's to choose");

        McpToolResult result = toolOver(page, new Saved()).execute(call);

        assertFalse(result.isError());
        JsonObject asked = page.asked.get(0);
        assertEquals("openSession", asked.get("action").getAsString());
        assertEquals("aaaa-1", asked.get("sessionId").getAsString());
        assertEquals("First one", asked.get("title").getAsString());
        assertEquals(FRONT, asked.get("folder").getAsString());
    }

    @Test
    void aConversationTooOldToBeListedStillOpensWithoutATitle() {
        Page page = new Page("{\"ok\":true,\"tab\":{\"id\":\"tab5\"}}");
        JsonObject call = args("claudeCodeView", "openSession");
        call.addProperty("sessionId", "old-0");

        McpToolResult result = toolOver(page, new Saved()).execute(call);

        assertFalse(result.isError());
        assertEquals("", page.asked.get(0).get("title").getAsString());
    }

    @Test
    void aConversationThatIsListedOpensWhereverItsFileIs() {
        Page page = new Page("{\"ok\":true,\"tab\":{\"id\":\"tab5\"}}");
        Saved saved = new Saved();
        saved.filesFound = false;
        JsonObject call = args("claudeCodeView", "openSession");
        call.addProperty("sessionId", "bbbb-2");

        McpToolResult result = toolOver(page, saved).execute(call);

        assertFalse(result.isError());
        assertEquals("Second one", page.asked.get(0).get("title").getAsString());
    }

    @Test
    void aTitleLosesTheWrappersThatAreNotTheUsersWords() {
        assertEquals("Fix the parser", ClaudeCodeViewModule.title("Fix the parser"));
        assertEquals("Fix it", ClaudeCodeViewModule.title("<system-reminder>be brief</system-reminder>Fix it"));
        assertEquals("Real question",
                ClaudeCodeViewModule.title("<ide_something a=\"b\">junk\nmore</ide_something>Real question"));
        assertEquals("Real question", ClaudeCodeViewModule.title("<ide_context file=\"a.txt\"/>Real question"));
        assertEquals("/usage", ClaudeCodeViewModule.title("<command-name>/usage</command-name>"
                + "<command-stdout>42%</command-stdout><command-contents>x</command-contents>"));
        assertEquals("(untitled)", ClaudeCodeViewModule.title("<system-reminder>only this</system-reminder>"));
        assertEquals("(untitled)", ClaudeCodeViewModule.title(""));
    }

    @Test
    void theListAndTheTabCarryTheCleanedTitle() {
        Saved saved = new Saved();
        saved.listed = "[{\"sessionId\":\"aaaa-1\",\"display\":\"<system-reminder>x</system-reminder>First one\","
                + "\"timestamp\":\"2026-10-05T09:00:00Z\"}]";
        Page page = new Page("{\"ok\":true,\"tab\":{\"id\":\"tab5\"}}");
        JsonObject open = args("claudeCodeView", "openSession");
        open.addProperty("sessionId", "aaaa-1");

        JsonObject list = JsonParser.parseString(text(toolOver(page, saved)
                .execute(args("claudeCodeView", "listHistory")))).getAsJsonObject();
        toolOver(page, saved).execute(open);

        assertEquals("First one", list.getAsJsonArray("sessions").get(0).getAsJsonObject().get("title").getAsString());
        assertEquals("First one", page.asked.get(0).get("title").getAsString());
    }

    @Test
    void aConversationThatIsNotThereIsRefusedWithoutAskingThePage() {
        Page page = new Page("{\"ok\":true}");
        JsonObject missing = args("claudeCodeView", "openSession");
        missing.addProperty("sessionId", "zzzz-9");
        JsonObject elsewhere = args("claudeCodeView", "openSession");
        elsewhere.addProperty("sessionId", "aaaa-1");
        elsewhere.addProperty("folder", OTHER);
        JsonObject notAnId = args("claudeCodeView", "openSession");
        notAnId.addProperty("sessionId", "../../etc/passwd");

        assertTrue(toolOver(page, new Saved()).execute(missing).isError());
        assertTrue(toolOver(page, new Saved()).execute(elsewhere).isError());
        assertTrue(toolOver(page, new Saved()).execute(notAnId).isError());
        assertTrue(toolOver(page, new Saved()).execute(args("claudeCodeView", "openSession")).isError());
        assertTrue(page.asked.isEmpty());
    }

    @Test
    void aCallWithoutAModuleIsRefusedAndNamesThem() {
        Page page = new Page("{\"ok\":true}");

        McpToolResult result = toolOver(page).execute(args(null, "listTabs"));

        assertTrue(result.isError());
        assertTrue(text(result).contains("claudeCodeView"));
        assertTrue(page.asked.isEmpty());
    }

    @Test
    void anUnknownModuleIsRefused() {
        Page page = new Page("{\"ok\":true}");

        McpToolResult result = toolOver(page).execute(args("claudeTerminalView", "listTabs"));

        assertTrue(result.isError());
        assertTrue(page.asked.isEmpty());
    }

    @Test
    void anUnknownActionIsRefusedWithoutAskingThePage() {
        Page page = new Page("{\"ok\":true}");

        McpToolResult result = toolOver(page).execute(args("claudeCodeView", "renameTab"));

        assertTrue(result.isError());
        assertTrue(text(result).contains("configureTab"));
        assertTrue(page.asked.isEmpty());
    }

    @Test
    void aPromptReachesThePageWithItsTab() {
        Page page = new Page("{\"ok\":true,\"queued\":false,\"tab\":{\"id\":\"tab3\"}}");
        JsonObject call = args("claudeCodeView", "sendPrompt");
        call.addProperty("tabId", "tab3");
        call.addProperty("prompt", "run the tests");

        McpToolResult result = toolOver(page).execute(call);

        assertFalse(result.isError());
        JsonObject asked = page.asked.get(0);
        assertEquals("sendPrompt", asked.get("action").getAsString());
        assertEquals("tab3", asked.get("tabId").getAsString());
        assertEquals("run the tests", asked.get("prompt").getAsString());
    }

    @Test
    void aRemoteControlSwitchReachesThePageAsABoolean() {
        Page page = new Page("{\"ok\":true,\"tab\":{\"id\":\"tab3\",\"remoteControl\":\"connecting\"}}");
        JsonObject call = args("claudeCodeView", "remoteControl");
        call.addProperty("tabId", "tab3");
        call.addProperty("enabled", true);

        McpToolResult result = toolOver(page).execute(call);

        assertFalse(result.isError());
        JsonObject asked = page.asked.get(0);
        assertEquals("remoteControl", asked.get("action").getAsString());
        assertTrue(asked.get("enabled").getAsJsonPrimitive().isBoolean());
        assertTrue(asked.get("enabled").getAsBoolean());
    }

    @Test
    void closingATabIsAskedOfThePage() {
        Page page = new Page("{\"ok\":true,\"closed\":\"tab3\",\"tabs\":[]}");
        JsonObject call = args("claudeCodeView", "closeTab");
        call.addProperty("tabId", "tab3");

        McpToolResult result = toolOver(page).execute(call);

        assertFalse(result.isError());
        assertEquals("closeTab", page.asked.get(0).get("action").getAsString());
        assertEquals("tab3", JsonParser.parseString(text(result)).getAsJsonObject().get("closed").getAsString());
    }

    @Test
    void aCommandReachesThePageWithItsTab() {
        Page page = new Page("{\"ok\":true,\"command\":\"/review\",\"tab\":{\"id\":\"tab3\"}}");
        JsonObject call = args("claudeCodeView", "runCommand");
        call.addProperty("tabId", "tab3");
        call.addProperty("command", "/review src/x");
        call.addProperty("stray", "x");

        McpToolResult result = toolOver(page).execute(call);

        assertFalse(result.isError());
        JsonObject asked = page.asked.get(0);
        assertEquals("runCommand", asked.get("action").getAsString());
        assertEquals("tab3", asked.get("tabId").getAsString());
        assertEquals("/review src/x", asked.get("command").getAsString());
        assertFalse(asked.has("stray"));
    }

    @Test
    void theSchemaDescribesTheCommand() {
        JsonObject props = new ClaudeCodeEclipseTool().inputSchema().getAsJsonObject("properties");

        assertTrue(props.has("command"));
        assertEquals("string", props.getAsJsonObject("command").get("type").getAsString());
    }

    @Test
    void theFolderOfANewTabReachesThePageAsItWasGiven() {
        Page page = new Page("{\"ok\":true,\"tab\":{\"id\":\"tab4\"}}");
        JsonObject call = args("claudeCodeView", "newTab");
        call.addProperty("folder", "C:\\work\\api");

        McpToolResult result = toolOver(page).execute(call);

        assertFalse(result.isError());
        JsonObject asked = page.asked.get(0);
        assertEquals("newTab", asked.get("action").getAsString());
        assertEquals("C:\\work\\api", asked.get("folder").getAsString());
    }

    @Test
    void theSchemaDescribesTheFolderOfANewTab() {
        JsonObject props = new ClaudeCodeEclipseTool().inputSchema().getAsJsonObject("properties");

        assertTrue(props.has("folder"));
        assertEquals("string", props.getAsJsonObject("folder").get("type").getAsString());
    }

    @Test
    void theActionAndItsSettingsReachThePageAndNothingElseDoes() {
        Page page = new Page("{\"ok\":true,\"tab\":{\"id\":\"tab2\"}}");
        JsonObject call = args("claudeCodeView", "configureTab");
        call.addProperty("tabId", "tab2");
        call.addProperty("model", "opus");
        call.addProperty("effort", "max");
        call.addProperty("thinking", false);
        call.addProperty("mode", "plan");
        call.addProperty("stray", "x");

        toolOver(page).execute(call);

        JsonObject asked = page.asked.get(0);
        assertEquals("configureTab", asked.get("action").getAsString());
        assertEquals("tab2", asked.get("tabId").getAsString());
        assertEquals("opus", asked.get("model").getAsString());
        assertEquals("max", asked.get("effort").getAsString());
        assertFalse(asked.get("thinking").getAsBoolean());
        assertEquals("plan", asked.get("mode").getAsString());
        assertFalse(asked.has("module"));
        assertFalse(asked.has("stray"));
    }

    @Test
    void whatThePageDidComesBackAsTheResult() {
        Page page = new Page("{\"ok\":true,\"tab\":{\"id\":\"tab2\",\"effort\":\"max\"},\"notes\":[\"n\"]}");

        McpToolResult result = toolOver(page).execute(args("claudeCodeView", "newTab"));

        assertFalse(result.isError());
        JsonObject out = JsonParser.parseString(text(result)).getAsJsonObject();
        assertEquals("tab2", out.getAsJsonObject("tab").get("id").getAsString());
        assertEquals(1, out.getAsJsonArray("notes").size());
        assertFalse(out.has("ok"));
    }

    @Test
    void aRefusalFromThePageIsAnErrorCarryingItsReason() {
        Page page = new Page("{\"ok\":false,\"error\":\"Unknown effort 'ultra'.\"}");

        McpToolResult result = toolOver(page).execute(args("claudeCodeView", "configureTab"));

        assertTrue(result.isError());
        assertEquals("Unknown effort 'ultra'.", text(result));
    }

    @Test
    void noPageToAskMeansTheViewIsNotOpen() {
        Page page = new Page(null);

        McpToolResult result = toolOver(page).execute(args("claudeCodeView", "listTabs"));

        assertTrue(result.isError());
        assertTrue(text(result).contains("eclipseShowView"));
    }

    @Test
    void withTheViewSwitchedOffNothingIsAskedOfThePage() {
        Page page = new Page("{\"ok\":true}");
        ClaudeCodeEclipseTool tool = new ClaudeCodeEclipseTool(new ClaudeCodeViewModule(page::call, () -> true));

        McpToolResult result = tool.execute(args("claudeCodeView", "listTabs"));

        assertTrue(result.isError());
        assertTrue(text(result).contains("Exclusively use terminal"));
        assertTrue(page.asked.isEmpty());
    }

    @Test
    void aReplyThatIsNotJsonIsAnErrorRatherThanACrash() {
        Page page = new Page("undefined");

        McpToolResult result = toolOver(page).execute(args("claudeCodeView", "listTabs"));

        assertTrue(result.isError());
    }
}
