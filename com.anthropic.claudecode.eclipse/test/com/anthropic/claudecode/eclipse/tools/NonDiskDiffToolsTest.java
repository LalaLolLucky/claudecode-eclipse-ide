package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.google.gson.JsonObject;

/**
 * The diff tools read and write files by disk path, so a file another file system keeps (ABAP's
 * {@code semanticfs:}, #153) is refused with a plain message, before anything touches the
 * workbench: this runs with none. It used to fail on the path, or be taken for a new file.
 */
class NonDiskDiffToolsTest {

    private static final String ABAP = "semanticfs:/E10_900_user_de/.adt/programs/zdemo.prog";

    private static void assertRefused(McpToolResult result) {
        assertTrue(result.isError(), "refused");
        String text = result.toJson().toString();
        assertTrue(text.contains("is not a file on disk"), text);
        assertTrue(text.contains(ABAP), text);
    }

    @Test
    void openDiffRefusesAFileThatIsNotOnDisk() {
        JsonObject params = new JsonObject();
        params.addProperty("old_file_path", ABAP);
        params.addProperty("new_file_contents", "REPORT zdemo.");
        assertRefused(new OpenDiffTool().execute(params));
    }

    @Test
    void acceptDiffRefusesAFileThatIsNotOnDisk() {
        JsonObject params = new JsonObject();
        params.addProperty("file_path", ABAP);
        assertRefused(new AcceptDiffTool().execute(params));
    }
}
