package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

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

        assertEquals(1, modules.size());
        assertEquals("claudeCodeView", modules.get(0).getAsString());
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
