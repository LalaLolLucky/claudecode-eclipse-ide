package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

import java.nio.file.Files;
import java.nio.file.Path;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

class ProposedChangeTest {

    /** Replacing old by new, once, in orig: what the CLI's Edit does. */
    private static String apply(String orig, String[] span) {
        int i = orig.indexOf(span[0]);
        assertEquals(-1, orig.indexOf(span[0], i + 1), "the old text is found once: " + span[0]);
        return orig.substring(0, i) + span[1] + orig.substring(i + span[0].length());
    }

    private static void roundTrips(String orig, String edited) {
        assertEquals(edited, apply(orig, ProposedChange.minimalEdit(orig, edited)));
    }

    @Test
    void anEditIsTheLinesThatChanged() {
        assertArrayEquals(new String[] { "second", "SECOND" },
                ProposedChange.minimalEdit("first\nsecond\nthird\n", "first\nSECOND\nthird\n"));
    }

    @Test
    void nothingChangedIsNoEdit() {
        assertNull(ProposedChange.minimalEdit("same", "same"));
    }

    @Test
    void aRepeatedLineIsWidenedUntilItIsFoundOnce() {
        String orig = "a\nx\nb\nx\nc\n";
        String edited = "a\nx\nb\ny\nc\n";
        String[] span = ProposedChange.minimalEdit(orig, edited);
        assertEquals(edited, apply(orig, span));
    }

    @Test
    void anyKindOfChangeGivesTheEditedTextBack() {
        roundTrips("one\ntwo\nthree\n", "zero\none\ntwo\nthree\n");         // insert first
        roundTrips("one\ntwo\nthree\n", "one\ntwo\nthree\nfour\n");         // append
        roundTrips("one\ntwo\nthree\n", "one\nthree\n");                     // delete
        roundTrips("one\ntwo\nthree\n", "one\ntwo\n\nthree\n");              // blank line
        roundTrips("x\nx\nx\n", "x\nx\n");                                   // all alike
        roundTrips("x\nx\nx\n", "x\ny\nx\n");
        roundTrips("no newline", "no newline at all");
        roundTrips("a\r\nb\r\nc\r\n", "a\r\nB\r\nc\r\n");
        roundTrips("only", "");
    }

    @Test
    void aFileThatDoesNotExistYetIsTakenWhole() {
        assertArrayEquals(new String[] { "", "new text" }, ProposedChange.minimalEdit("", "new text"));
    }

    @Test
    void aWriteTakesTheEditedTextWhole(@TempDir Path dir) throws Exception {
        Path f = dir.resolve("w.txt");
        Files.writeString(f, "old");
        JsonObject in = JsonParser.parseString(
                "{\"file_path\":" + new com.google.gson.Gson().toJson(f.toString()) + ",\"content\":\"new\"}").getAsJsonObject();
        ProposedChange c = ProposedChange.of("Write", in);
        assertEquals("old", c.original);
        assertEquals("new", c.proposed);
        JsonObject out = c.editedInput("Write", in, "edited");
        assertEquals("edited", out.get("content").getAsString());
        assertEquals(in.get("file_path"), out.get("file_path"));
        assertNull(c.editedInput("Write", in, "new"), "unchanged: nothing to send back");
    }

    @Test
    void anEditTakesTheLinesThatDifferFromTheFileAndIsNotReplaceAll(@TempDir Path dir) throws Exception {
        Path f = dir.resolve("a.txt");
        Files.writeString(f, "one two\nsecond line\n");
        JsonObject in = new JsonObject();
        in.addProperty("file_path", f.toString());
        in.addProperty("old_string", "two");
        in.addProperty("new_string", "2");
        ProposedChange c = ProposedChange.of("Edit", in);
        assertEquals("one 2\nsecond line\n", c.proposed);
        JsonObject out = c.editedInput("Edit", in, "one 2\nSECOND LINE\n");
        // The span runs from the file as it is to the text as edited, so it holds the call's own
        // change (two to 2) as well as the reader's.
        assertEquals("one two" + "\n" + "second line", out.get("old_string").getAsString());
        assertEquals("one 2" + "\n" + "SECOND LINE", out.get("new_string").getAsString());
        assertEquals(false, out.get("replace_all").getAsBoolean());
    }

    @Test
    void whatIsNotAFileEditProposesNothing() {
        assertNull(ProposedChange.of("Bash", JsonParser.parseString("{\"command\":\"ls\"}")));
        assertNull(ProposedChange.of("Read", JsonParser.parseString("{\"file_path\":\"x\"}")));
    }
}
