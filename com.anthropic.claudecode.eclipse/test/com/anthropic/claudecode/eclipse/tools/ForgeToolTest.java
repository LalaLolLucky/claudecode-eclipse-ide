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
 * Tests for what {@link ForgeTool} asks of the build tool and what it makes of the answers,
 * with the build tool replaced by a function so no workspace is needed.
 */
class ForgeToolTest {

    /** A build tool that records its calls and answers each with the next reply. */
    private static final class Builds {
        final List<JsonObject> asked = new ArrayList<>();
        final List<McpToolResult> replies = new ArrayList<>();

        McpToolResult call(JsonObject args) {
            asked.add(args);
            return replies.get(asked.size() - 1);
        }
    }

    private static McpToolResult built(boolean autoBuild, int errors, long elapsedMs) {
        JsonObject r = new JsonObject();
        r.addProperty("scope", "workspace");
        r.addProperty("elapsedMs", elapsedMs);
        r.addProperty("autoBuildEnabled", autoBuild);
        r.addProperty("errorCount", errors);
        r.addProperty("warningCount", 1);
        r.add("projects", new JsonArray());
        r.add("errors", new JsonArray());
        return McpToolResult.success(r);
    }

    private static JsonObject json(McpToolResult result) {
        return JsonParser.parseString(result.content().get(0).getAsJsonObject().get("text").getAsString())
                .getAsJsonObject();
    }

    @Test
    void runsACleanRebuildThenAFullBuildEachRefreshedFirst() {
        Builds builds = new Builds();
        builds.replies.add(built(false, 4, 300));
        builds.replies.add(built(false, 0, 200));
        JsonObject params = new JsonObject();
        JsonArray projects = new JsonArray();
        projects.add("core");
        params.add("projects", projects);

        McpToolResult result = new ForgeTool(builds::call, line -> { }).execute(params);

        assertFalse(result.isError());
        assertEquals(2, builds.asked.size());
        assertEquals("clean-rebuild", builds.asked.get(0).get("mode").getAsString());
        assertEquals("rebuild", builds.asked.get(1).get("mode").getAsString());
        for (JsonObject asked : builds.asked) {
            assertTrue(asked.get("refresh").getAsBoolean());
            assertEquals("core", asked.getAsJsonArray("projects").get(0).getAsString());
        }
        JsonObject out = json(result);
        assertEquals("[\"refresh\",\"clean\",\"build\",\"refresh\",\"build\"]", out.get("steps").toString());
        assertEquals(500, out.get("elapsedMs").getAsLong());
        assertEquals(4, out.getAsJsonObject("firstPass").get("errorCount").getAsInt());
        assertEquals(0, out.getAsJsonObject("secondPass").get("errorCount").getAsInt());
        assertEquals(0, out.get("errorCount").getAsInt());
    }

    @Test
    void saysNothingAboutAutoBuildWhileItIsOff() {
        Builds builds = new Builds();
        builds.replies.add(built(false, 0, 1));
        builds.replies.add(built(false, 0, 1));

        JsonObject out = json(new ForgeTool(builds::call, line -> { }).execute(new JsonObject()));

        assertFalse(out.get("autoBuildEnabled").getAsBoolean());
        assertFalse(out.has("warning"));
    }

    @Test
    void warnsAboutAutoBuildAndStillRunsBothPasses() {
        Builds builds = new Builds();
        builds.replies.add(built(true, 0, 1));
        builds.replies.add(built(true, 0, 1));

        McpToolResult result = new ForgeTool(builds::call, line -> { }).execute(new JsonObject());

        assertFalse(result.isError());
        assertEquals(2, builds.asked.size());
        JsonObject out = json(result);
        assertTrue(out.get("autoBuildEnabled").getAsBoolean());
        assertEquals(ForgeTool.AUTO_BUILD_WARNING, out.get("warning").getAsString());
    }

    @Test
    void stopsWhenTheFirstPassIsRefused() {
        Builds builds = new Builds();
        builds.replies.add(McpToolResult.error("Cannot build: nope (no such project)"));

        McpToolResult result = new ForgeTool(builds::call, line -> { }).execute(new JsonObject());

        assertTrue(result.isError());
        assertEquals(1, builds.asked.size());
    }
}
