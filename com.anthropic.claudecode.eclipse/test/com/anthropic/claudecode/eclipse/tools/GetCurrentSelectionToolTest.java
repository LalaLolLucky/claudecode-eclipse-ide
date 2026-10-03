package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.eclipse.jface.text.Document;
import org.eclipse.jface.text.IDocument;
import org.eclipse.jface.text.TextSelection;
import org.junit.jupiter.api.Test;

import com.anthropic.claudecode.eclipse.tools.GetCurrentSelectionTool.Range;
import com.google.gson.JsonObject;

/**
 * Tests for {@link GetCurrentSelectionTool#rangeOf}: where a selection starts and ends, as 1-based
 * lines and 0-based columns in their line, the numbers getLatestSelection reports for the same
 * selection. Plain {@link Document}s, so no editor is needed.
 */
class GetCurrentSelectionToolTest {

    private static Range rangeOf(IDocument doc, String selected) {
        int offset = doc.get().indexOf(selected);
        return GetCurrentSelectionTool.rangeOf(new TextSelection(doc, offset, selected.length()), doc);
    }

    @Test
    void aSelectionInsideOneLineGivesItsColumns() {
        Document doc = new Document("first\nsecond third\n");
        assertEquals(new Range(2, 7, 2, 12), rangeOf(doc, "third"));
    }

    @Test
    void aSelectionStartingMidLineAndEndingAtALineEndGivesBothColumns() {
        Document doc = new Document("- Read no longer shows the file's contents in the conversation (#147).\r\n"
                + "- Grep and Glob show the folder they searched (#147).\r\n"
                + "- Minor UI fixes (#147).");
        assertEquals(new Range(1, 43, 3, 24), rangeOf(doc, "in the conversation (#147).\r\n"
                + "- Grep and Glob show the folder they searched (#147).\r\n"
                + "- Minor UI fixes (#147)."));
    }

    @Test
    void aSelectionTakingTheLineBreakEndsAtColumnZeroOfTheNextLine() {
        Document doc = new Document("one\ntwo\nthree\n");
        assertEquals(new Range(2, 0, 3, 0), rangeOf(doc, "two\n"));
    }

    @Test
    void crlfLineBreaksCountAsOneBreak() {
        Document doc = new Document("ab\r\ncd\r\n");
        assertEquals(new Range(1, 1, 2, 1), rangeOf(doc, "b\r\nc"));
    }

    // ---- describe: what the tool answers ----

    @Test
    void aBareCaretIsReportedAsEmptyWithItsFileAndPosition() {
        Document doc = new Document("first\nsecond third\n");
        TextSelection caret = new TextSelection(doc, doc.get().indexOf("third"), 0);

        JsonObject result = GetCurrentSelectionTool.describe("C:/ws/A.java", caret, doc);

        assertEquals(true, result.get("isEmpty").getAsBoolean());
        assertEquals("", result.get("text").getAsString());
        assertEquals("C:/ws/A.java", result.get("filePath").getAsString());
        assertEquals(2, result.get("startLine").getAsInt());
        assertEquals(2, result.get("endLine").getAsInt());
        assertEquals(7, result.get("startColumn").getAsInt());
        assertEquals(7, result.get("endColumn").getAsInt());
    }

    @Test
    void highlightedTextIsReportedAsASelection() {
        Document doc = new Document("first\nsecond third\n");
        TextSelection selection = new TextSelection(doc, doc.get().indexOf("third"), 5);

        JsonObject result = GetCurrentSelectionTool.describe("C:/ws/A.java", selection, doc);

        assertEquals(false, result.get("isEmpty").getAsBoolean());
        assertEquals("third", result.get("text").getAsString());
        assertEquals(7, result.get("startColumn").getAsInt());
        assertEquals(12, result.get("endColumn").getAsInt());
    }

    @Test
    void aFileWithNoPathIsReportedWithAnEmptyPath() {
        Document doc = new Document("abc");
        JsonObject result = GetCurrentSelectionTool.describe(null, new TextSelection(doc, 0, 1), doc);
        assertEquals("", result.get("filePath").getAsString());
    }

    @Test
    void withoutADocumentOnlyTheLinesAreKnown() {
        Document doc = new Document("first\nsecond third\n");
        TextSelection selection = new TextSelection(doc, doc.get().indexOf("third"), 5);
        assertEquals(new Range(2, 0, 2, 0), GetCurrentSelectionTool.rangeOf(selection, null));
    }

    @Test
    void offsetsTheDocumentNoLongerHasFallBackToTheSelectionsLines() {
        Document before = new Document("first\nsecond third\n");
        TextSelection selection = new TextSelection(before, before.get().indexOf("third"), 5);
        assertEquals(new Range(2, 0, 2, 0), GetCurrentSelectionTool.rangeOf(selection, new Document("x")));
    }
}
