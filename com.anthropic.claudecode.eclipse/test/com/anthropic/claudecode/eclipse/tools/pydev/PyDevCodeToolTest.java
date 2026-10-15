package com.anthropic.claudecode.eclipse.tools.pydev;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/** Tests for what pydevCode refuses before it asks PyDev anything. */
class PyDevCodeToolTest {

	private static JsonObject call(String action) {
		JsonObject j = new JsonObject();
		if (action != null) j.addProperty("action", action);
		return j;
	}

	private static String text(McpToolResult result) {
		return result.content().get(0).getAsJsonObject().get("text").getAsString();
	}

	@Test
	void theSchemaOffersDefinitionAndReferences() {
		JsonObject schema = new PyDevCodeTool().inputSchema();
		JsonArray actions = schema.getAsJsonObject("properties").getAsJsonObject("action").getAsJsonArray("enum");

		assertEquals(2, actions.size());
		assertEquals("definition", actions.get(0).getAsString());
		assertEquals("references", actions.get(1).getAsString());
		assertEquals("pydevCode", new PyDevCodeTool().toolName());
	}

	@Test
	void anActionItDoesNotHaveIsRefused() {
		McpToolResult result = new PyDevCodeTool().execute(call("rename"));

		assertTrue(result.isError());
		assertTrue(text(result).contains("definition") && text(result).contains("references"), text(result));
	}

	@Test
	void aFileIsRequired() {
		McpToolResult none = new PyDevCodeTool().execute(call("definition"));

		assertTrue(none.isError() && text(none).contains("'file'"), text(none));
	}

	@Test
	void aPositionIsRequired() {
		JsonObject noLine = call("definition");
		noLine.addProperty("file", "/NoSuchProject/nowhere.py");

		McpToolResult result = new PyDevCodeTool().execute(noLine);

		assertTrue(result.isError() && text(result).contains("'line'"), text(result));
	}
}
