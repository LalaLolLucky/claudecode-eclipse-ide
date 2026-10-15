package com.anthropic.claudecode.eclipse.ui.handlers;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

import java.nio.file.Paths;

import org.eclipse.core.resources.IMarker;
import org.junit.jupiter.api.Test;

/**
 * Tests for the text "Send to Claude Terminal" and "Send to Claude Code" put at Claude's prompt
 * from the Problems and Console views.
 */
class SendToClaudeTextTest {

    private static final String FOLDER = Paths.get("work").toAbsolutePath().normalize().toString();

    @Test
    void aProblemIsItsFileAtItsLineThenItsSeverityAndMessage() {
        assertEquals("@src/A.java#L12 Error: x cannot be resolved",
                SendToClaudeText.problem("src/A.java", 12, IMarker.SEVERITY_ERROR, "x cannot be resolved"));
        assertEquals("@pom.xml#L3 Warning: Overriding managed version",
                SendToClaudeText.problem("pom.xml", 3, IMarker.SEVERITY_WARNING, "Overriding managed version"));
        assertEquals("@README.md#L1 Info: Spelling",
                SendToClaudeText.problem("README.md", 1, IMarker.SEVERITY_INFO, "Spelling"));
    }

    @Test
    void aProblemWithoutALineOrAFileSaysWhatItHas() {
        assertEquals("@app Error: Build path is incomplete",
                SendToClaudeText.problem("app", -1, IMarker.SEVERITY_ERROR, "Build path is incomplete"));
        assertEquals("Warning: Unbound classpath container",
                SendToClaudeText.problem(null, 4, IMarker.SEVERITY_WARNING, "Unbound classpath container"));
    }

    @Test
    void aMessageOverSeveralLinesIsKeptOnOne() {
        assertEquals("@A.java#L2 Error: first second",
                SendToClaudeText.problem("A.java", 2, IMarker.SEVERITY_ERROR, "first\r\n  second"));
    }

    @Test
    void aPathUnderTheFolderIsMentionedRelativeToItAndAnyOtherInFull() {
        String inside = Paths.get(FOLDER, "src", "A.java").toString();
        String outside = Paths.get(FOLDER).getParent().resolve("other").resolve("B.java").toString();

        assertEquals("src/A.java", SendToClaudeText.mentionPath(inside, FOLDER));
        assertEquals(asMentioned(outside), SendToClaudeText.mentionPath(outside, FOLDER));
        assertEquals(asMentioned(inside), SendToClaudeText.mentionPath(inside, null));
    }

    /** A full path as a mention spells it: forward slashes, quoted when it holds a space (as
     *  this machine's own folders may). */
    private static String asMentioned(String path) {
        String slashed = path.replace('\\', '/');
        return slashed.contains(" ") ? '"' + slashed + '"' : slashed;
    }

    @Test
    void aPathWithASpaceIsQuotedAsTheCliReadsIt() {
        String spaced = Paths.get(FOLDER, "my src", "A.java").toString();

        assertEquals("\"my src/A.java\"", SendToClaudeText.mentionPath(spaced, FOLDER));
        assertEquals("@\"my src/A.java\"#L4 Error: e",
                SendToClaudeText.problem(SendToClaudeText.mentionPath(spaced, FOLDER), 4, IMarker.SEVERITY_ERROR, "e"));
    }

    @Test
    void consoleTextIsTheSelectionOrElseTheLineTheCaretIsOn() {
        String doc = "first line\r\nException in thread \"main\"\r\n\tat A.main(A.java:3)\r\n";

        assertEquals("Exception in thread \"main\"\r\n\tat A.main(A.java:3)",
                SendToClaudeText.consoleText(doc, 12, 48));
        assertEquals("Exception in thread \"main\"", SendToClaudeText.consoleText(doc, 20, 0));
        assertEquals("first line", SendToClaudeText.consoleText(doc, 0, 0));
        assertEquals("\tat A.main(A.java:3)", SendToClaudeText.consoleText(doc, doc.length() - 3, 0));
    }

    @Test
    void anEmptyLineOrNothingAtAllIsNothingToSend() {
        assertNull(SendToClaudeText.consoleText("a\n\nb", 2, 0));
        assertNull(SendToClaudeText.consoleText("", 0, 0));
        assertNull(SendToClaudeText.consoleText("   ", 1, 0));
        assertNull(SendToClaudeText.consoleText(null, 0, 0));
        assertNull(SendToClaudeText.consoleText("abc", 5, 0));
    }

    @Test
    void oneLineGoesToTheTerminalAsItIsAndSeveralAsOnePaste() {
        assertEquals("@A.java#L1 Error: e", SendToClaudeText.forTerminal("@A.java#L1 Error: e"));
        assertEquals("\u001b[200~one\rtwo\u001b[201~", SendToClaudeText.forTerminal("one\r\ntwo"));
        assertEquals("\u001b[200~one\rtwo\u001b[201~", SendToClaudeText.forTerminal("one\ntwo\n"));
    }
}
