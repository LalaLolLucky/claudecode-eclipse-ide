package com.anthropic.claudecode.eclipse.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.fail;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Set;

import org.junit.jupiter.api.Test;

/**
 * Tests for reading which flags a CLI program knows out of its bytes, on in-memory
 * programs so no installed CLI is needed.
 */
class CliFlagSupportTest {

    private static final List<String> ASKED = List.of("--mcp-config", "--remote-control",
            "--allow-dangerously-skip-permissions");

    private static Set<String> scan(String program, int chunk) throws IOException {
        return CliFlagSupport.scan(
                new ByteArrayInputStream(program.getBytes(StandardCharsets.ISO_8859_1)), ASKED, chunk);
    }

    @Test
    void aFlagTheProgramNamesIsFound() throws IOException {
        assertEquals(Set.of("--mcp-config"),
                scan("\0\0junk\"--mcp-config <configs...>\"more\0", 4096));
    }

    @Test
    void aProgramThatNamesNoneOfThemKnowsNone() throws IOException {
        assertEquals(Set.of(), scan("an old cli with --print and --model only", 4096));
    }

    @Test
    void aLongerFlagThatOnlyStartsTheSameIsNotTheFlag() throws IOException {
        assertEquals(Set.of(), scan("\"--remote-control-session-name-prefix <prefix>\"", 4096));
    }

    @Test
    void theFlagIsFoundBesideALongerOneThatStartsTheSame() throws IOException {
        assertEquals(Set.of("--remote-control"),
                scan("--remote-control-session-name-prefix,--remote-control [name]", 4096));
    }

    @Test
    void aFlagAtTheVeryEndOfTheProgramIsFound() throws IOException {
        assertEquals(Set.of("--remote-control"), scan("xx --remote-control", 4096));
    }

    @Test
    void aFlagSplitAcrossTwoReadsIsStillFound() throws IOException {
        String program = "0123456789" + "--allow-dangerously-skip-permissions" + " tail";

        // Every chunk size puts the boundary somewhere else inside the flag.
        for (int chunk = 1; chunk <= program.length(); chunk++) {
            assertEquals(Set.of("--allow-dangerously-skip-permissions"), scan(program, chunk), "chunk " + chunk);
        }
    }

    @Test
    void aLongerFlagSplitAcrossTwoReadsIsStillNotTheFlag() throws IOException {
        String program = "0123456789" + "--remote-control-session-name-prefix" + " tail";

        for (int chunk = 1; chunk <= program.length(); chunk++) {
            assertEquals(Set.of(), scan(program, chunk), "chunk " + chunk);
        }
    }

    @Test
    void whatTheCoreConfirmsIsNotReadAgain() {
        Set<String> known = CliFlagSupport.knownTo(ASKED, flag -> true, () -> fail("the program was read"));

        assertEquals(Set.copyOf(ASKED), known);
    }

    @Test
    void whatTheCoreDoesNotConfirmIsReadFromTheProgram() {
        Set<String> known = CliFlagSupport.knownTo(ASKED, "--mcp-config"::equals,
                () -> Set.of("--mcp-config", "--remote-control"));

        assertEquals(Set.of("--mcp-config", "--remote-control"), known);
    }

    @Test
    void aCoreFromBeforeTheQuestionLeavesItToTheReading() {
        Set<String> known = CliFlagSupport.knownTo(ASKED, flag -> {
            throw new UnsatisfiedLinkError("cliSupportsFlag");
        }, () -> Set.of("--remote-control"));

        assertEquals(Set.of("--remote-control"), known);
    }

    @Test
    void aFlagNeitherOfThemFindsIsNotKnown() throws IOException {
        // What an older CLI looks like to both: the longer option, and not the shorter one.
        Set<String> read = scan("\"--remote-control-session-name-prefix <prefix>\"", 4096);

        assertEquals(Set.of(), CliFlagSupport.knownTo(ASKED, flag -> false, () -> read));
    }

    @Test
    void whereTheProgramCannotBeReadTheCoreAnswersAlone() {
        assertEquals(Set.of("--mcp-config"),
                CliFlagSupport.knownTo(ASKED, "--mcp-config"::equals, () -> null));
    }

    @Test
    void severalFlagsAreFoundInOnePass() throws IOException {
        assertEquals(Set.of("--mcp-config", "--remote-control", "--allow-dangerously-skip-permissions"),
                scan("--allow-dangerously-skip-permissions\0--remote-control\0--mcp-config\0", 7));
    }
}
