package com.anthropic.claudecode.eclipse.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayDeque;
import java.util.Deque;
import java.util.List;

import org.junit.jupiter.api.Test;

import com.google.gson.JsonParser;

class CliUpdateServiceTest {

    // ---- The line the banner shows -----------------------------------------

    @Test
    void theFailureLineIsShownRatherThanTheVersionTheUpdateStartedFrom() {
        String output = "Current version: 2.1.301\nChecking for updates to latest version...\n"
                + "Failed to install new version of claude: EBUSY: resource busy or locked\n"
                + "Try running \"claude doctor\" for diagnostics";
        assertEquals("Failed to install new version of claude: EBUSY: resource busy or locked",
                CliUpdateService.failureLine(output));
    }

    @Test
    void colourCodesAreNotPartOfTheLine() {
        assertEquals("Error: npm exited with code 1",
                CliUpdateService.failureLine("Current version: 2.1.301\n\u001B[31mError: npm exited with code 1\u001B[0m"));
    }

    @Test
    void withNoLineSayingWhatFailedTheLastOneIsShown() {
        assertEquals("Updates are disabled by your administrator",
                CliUpdateService.failureLine("Current version: 2.1.301\nUpdates are disabled by your administrator\n"));
        assertEquals("Current version: 2.1.301", CliUpdateService.failureLine("Current version: 2.1.301"));
    }

    @Test
    void nothingPrintedGivesAnEmptyLine() {
        assertEquals("", CliUpdateService.failureLine(""));
        assertEquals("", CliUpdateService.failureLine("  \n\n"));
        assertEquals("", CliUpdateService.failureLine(null));
    }

    @Test
    void aFailedResultCarriesItsReasonToThePageAndKeepsTheWholeOutput() {
        String output = "Current version: 2.1.301\nFailed to install new version of claude: EBUSY";
        var json = JsonParser.parseString(new CliUpdateService.Result(false, output).toJson()).getAsJsonObject();
        assertEquals("Failed to install new version of claude: EBUSY", json.get("reason").getAsString());
        assertEquals(output, json.get("output").getAsString());
    }

    @Test
    void aSuccessfulResultHasNoReason() {
        var json = JsonParser.parseString(new CliUpdateService.Result(true, "Successfully updated").toJson())
                .getAsJsonObject();
        assertFalse(json.has("reason"));
    }

    // ---- The native report -------------------------------------------------

    @Test
    void theReportSaysWhetherTheInstallIsNpms() {
        assertTrue(CliUpdateService.isNpmInstall("{\"kept\":[],\"npm\":true,\"removed\":[],\"setAside\":[]}"));
        assertFalse(CliUpdateService.isNpmInstall("{\"kept\":[],\"npm\":false,\"removed\":[],\"setAside\":[]}"));
        assertFalse(CliUpdateService.isNpmInstall("{\"npm\":false}"));
    }

    @Test
    void aReportThatCannotBeReadIsNotAnNpmInstall() {
        assertFalse(CliUpdateService.isNpmInstall(null));
        assertFalse(CliUpdateService.isNpmInstall(""));
        assertFalse(CliUpdateService.isNpmInstall("not json"));
        assertFalse(CliUpdateService.isNpmInstall("[true]"));
        assertFalse(CliUpdateService.isNpmInstall("{\"npm\":\"yes\"}"));
    }

    // ---- Windows: clear, update, and one retry -----------------------------

    /** Stands in for the native clearing, {@code claude update}, and the CLI's own check. */
    private static final class Windows {
        final boolean npm;
        final boolean cliStillRuns;
        final Deque<CliUpdateService.Result> results;
        int clears;
        int updates;

        Windows(boolean npm, boolean cliStillRuns, CliUpdateService.Result... results) {
            this.npm = npm;
            this.cliStillRuns = cliStillRuns;
            this.results = new ArrayDeque<>(List.of(results));
        }

        CliUpdateService.Result update() {
            return CliUpdateService.updateWindows(
                    () -> { clears++; return npm; },
                    () -> { updates++; return results.removeFirst(); },
                    () -> cliStillRuns);
        }
    }

    private static CliUpdateService.Result ok() {
        return new CliUpdateService.Result(true, "Successfully updated from 2.1.301 to version 2.1.306");
    }

    private static CliUpdateService.Result failed() {
        return new CliUpdateService.Result(false, "Current version: 2.1.301\nFailed to install new version of claude: EBUSY");
    }

    @Test
    void leftoversAreClearedBeforeTheFirstAttempt() {
        Windows w = new Windows(true, true, ok());
        CliUpdateService.Result first = w.results.peekFirst();
        assertSame(first, w.update());
        assertEquals(1, w.clears);
        assertEquals(1, w.updates);
    }

    @Test
    void anNpmUpdateThatFailsIsClearedAndRunOnceMore() {
        Windows w = new Windows(true, true, failed(), ok());
        assertTrue(w.update().ok);
        assertEquals(2, w.clears);
        assertEquals(2, w.updates);
    }

    @Test
    void aSecondFailureIsTheOneReported() {
        CliUpdateService.Result second = new CliUpdateService.Result(false, "Failed again");
        Windows w = new Windows(true, true, failed(), second);
        assertSame(second, w.update());
        assertEquals(2, w.updates);
    }

    @Test
    void anInstallThatIsNotNpmsIsNotRetried() {
        Windows w = new Windows(false, true, failed());
        assertFalse(w.update().ok);
        assertEquals(1, w.updates);
    }

    @Test
    void aPackageManagersAdviceIsNotRetried() {
        CliUpdateService.Result managed = new CliUpdateService.Result(false,
                "Claude is managed by winget.\nTo update, run:\n  winget upgrade Anthropic.ClaudeCode");
        Windows w = new Windows(true, true, managed);
        assertSame(managed, w.update());
        assertEquals(1, w.updates);
    }

    @Test
    void aFailureThatLeftNoWorkingCliIsNotRetried() {
        // `claude update` would run that broken binary, so a second attempt cannot start.
        Windows w = new Windows(true, false, failed());
        assertFalse(w.update().ok);
        assertEquals(1, w.updates);
    }
}
