package com.anthropic.claudecode.eclipse.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import java.util.Set;
import java.util.function.Predicate;

import org.junit.jupiter.api.Test;

import com.anthropic.claudecode.eclipse.ui.TerminalLaunchArgs.Options;
import com.google.gson.JsonObject;

/**
 * Tests for what the Claude Terminal adds to the {@code claude} command line for the
 * preferences that reach it, against a stand-in for "does the installed CLI know this flag".
 */
class TerminalLaunchArgsTest {

    private static final String CONFIG = "C:/state/terminal/mcp-10123.json";
    private static final String HIDDEN = "mcp__eclipse__openDiff,mcp__eclipse__acceptDiff,"
            + "mcp__eclipse__rejectDiff,mcp__eclipse__closeAllDiffTabs,"
            + "mcp__eclipse__askUserQuestion,mcp__eclipse__approvalPrompt";

    /** A CLI that knows every flag asked about. */
    private static final Predicate<String> CURRENT = flag -> true;

    private static final Options NOTHING = new Options(false, false, false, "");
    private static final Options TOOLS = new Options(true, false, false, "");
    private static final Options BYPASS = new Options(false, true, false, "");
    private static final Options REMOTE = new Options(false, false, true, "");
    private static final Options PLAN = new Options(false, false, false, "plan");

    private static Predicate<String> knowing(String... flags) {
        return Set.of(flags)::contains;
    }

    @Test
    void withEveryPreferenceOffNothingIsAdded() {
        assertEquals(List.of(), TerminalLaunchArgs.build(List.of(), NOTHING, CURRENT, CONFIG));
    }

    @Test
    void theToolServerComesWithTheToolsTheTerminalMustNotBeShown() {
        assertEquals(List.of("--mcp-config", CONFIG, "--disallowed-tools", HIDDEN),
                TerminalLaunchArgs.build(List.of(), TOOLS, CURRENT, CONFIG));
    }

    @Test
    void theIdeLinksOwnDiffToolIsNotAmongTheHiddenOnes() {
        // The CLI opens the diff editor itself through mcp__ide__openDiff; denying that
        // would take the Terminal's diff review away.
        for (String token : TerminalLaunchArgs.build(List.of(), TOOLS, CURRENT, CONFIG)) {
            assertFalse(token.contains("mcp__ide__"), token);
        }
    }

    @Test
    void aUsersOwnToolServersAndDenyListAreAddedToNotReplaced() {
        List<String> theirs = List.of("--mcp-config", "mine.json", "--disallowed-tools", "WebSearch");

        assertEquals(List.of("--mcp-config", CONFIG, "--disallowed-tools", HIDDEN),
                TerminalLaunchArgs.build(theirs, TOOLS, CURRENT, CONFIG));
    }

    @Test
    void aUserWhoAskedForOnlyTheirOwnServersGetsOnlyThose() {
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of("--strict-mcp-config"), TOOLS, CURRENT, CONFIG));
    }

    @Test
    void withoutAConfigFileTheToolServerIsLeftOut() {
        assertEquals(List.of(), TerminalLaunchArgs.build(List.of(), TOOLS, CURRENT, null));
    }

    @Test
    void aCliWithoutTheToolServerFlagStartsAsItAlwaysDid() {
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of(), TOOLS, knowing("--disallowed-tools"), CONFIG));
    }

    @Test
    void anOlderCliIsGivenTheDenyListUnderTheNameItKnows() {
        assertEquals(List.of("--mcp-config", CONFIG, "--disallowedTools", HIDDEN),
                TerminalLaunchArgs.build(List.of(), TOOLS, knowing("--mcp-config", "--disallowedTools"), CONFIG));
    }

    @Test
    void aCliThatCannotHideToolsIsNotGivenTheToolServer() {
        // Without a deny list the model could open and accept its own diffs.
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of(), TOOLS, knowing("--mcp-config"), CONFIG));
    }

    @Test
    void bypassIsAllowedNotEntered() {
        assertEquals(List.of("--allow-dangerously-skip-permissions"),
                TerminalLaunchArgs.build(List.of(), BYPASS, CURRENT, CONFIG));
    }

    @Test
    void aCliThatDoesNotKnowTheBypassFlagIsNotGivenIt() {
        assertEquals(List.of(), TerminalLaunchArgs.build(List.of(), BYPASS, knowing("--mcp-config"), CONFIG));
    }

    @Test
    void aUsersOwnBypassFlagIsNotRepeated() {
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of("--allow-dangerously-skip-permissions"), BYPASS, CURRENT, CONFIG));
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of("--dangerously-skip-permissions"), BYPASS, CURRENT, CONFIG));
    }

    @Test
    void remoteControlIsSwitchedOnAtLaunch() {
        assertEquals(List.of("--remote-control"),
                TerminalLaunchArgs.build(List.of(), REMOTE, CURRENT, CONFIG));
    }

    @Test
    void aCliThatDoesNotKnowRemoteControlIsNotGivenIt() {
        assertEquals(List.of(), TerminalLaunchArgs.build(List.of(), REMOTE, knowing("--mcp-config"), CONFIG));
    }

    @Test
    void aUsersOwnRemoteControlFlagIsNotRepeated() {
        assertEquals(List.of(), TerminalLaunchArgs.build(List.of("--rc"), REMOTE, CURRENT, CONFIG));
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of("--remote-control", "desk"), REMOTE, CURRENT, CONFIG));
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of("--remote-control=desk"), REMOTE, CURRENT, CONFIG));
    }

    @Test
    void remoteControlComesLastSoItsOptionalNameCannotSwallowAnotherArgument() {
        List<String> all = TerminalLaunchArgs.build(List.of(), new Options(true, true, true, ""), CURRENT, CONFIG);

        assertEquals("--remote-control", all.get(all.size() - 1));
        assertEquals("--allow-dangerously-skip-permissions", all.get(0));
        assertEquals(6, all.size());
    }

    @Test
    void theInitialPermissionModeIsTheModeTheSessionStartsIn() {
        assertEquals(List.of("--permission-mode", "plan"),
                TerminalLaunchArgs.build(List.of(), PLAN, CURRENT, CONFIG));
    }

    @Test
    void aCliThatDoesNotKnowThePermissionModeFlagIsNotGivenIt() {
        assertEquals(List.of(), TerminalLaunchArgs.build(List.of(), PLAN, knowing("--mcp-config"), CONFIG));
    }

    @Test
    void aUsersOwnStartingModeIsLeftAsTheyGaveIt() {
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of("--permission-mode", "acceptEdits"), PLAN, CURRENT, CONFIG));
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of("--permission-mode=acceptEdits"), PLAN, CURRENT, CONFIG));
        // Starts the session in bypass, so it is a starting mode of the user's own too.
        assertEquals(List.of(),
                TerminalLaunchArgs.build(List.of("--dangerously-skip-permissions"), PLAN, CURRENT, CONFIG));
    }

    @Test
    void thePermissionModeComesBeforeRemoteControlWhichStaysLast() {
        List<String> all = TerminalLaunchArgs.build(List.of(), new Options(true, true, true, "plan"), CURRENT, CONFIG);

        assertEquals("--remote-control", all.get(all.size() - 1));
        assertEquals("plan", all.get(all.indexOf("--permission-mode") + 1));
        assertEquals(8, all.size());
    }

    @Test
    void everyFlagThatMayBePassedIsOneTheCliIsAskedAbout() {
        for (String token : TerminalLaunchArgs.build(List.of(), new Options(true, true, true, "plan"), CURRENT, CONFIG)) {
            if (token.startsWith("--")) assertTrue(TerminalLaunchArgs.PROBED.contains(token), token);
        }
        assertTrue(TerminalLaunchArgs.PROBED.contains("--disallowedTools"));
    }

    @Test
    void thinkingLeftOnSaysNothingSoTheUsersOwnSettingStands() {
        JsonObject settings = new JsonObject();

        TerminalLaunchArgs.applyThinking(settings, true);

        assertFalse(settings.has("alwaysThinkingEnabled"));
    }

    @Test
    void thinkingSwitchedOffIsWrittenAsOff() {
        JsonObject settings = new JsonObject();

        TerminalLaunchArgs.applyThinking(settings, false);

        assertFalse(settings.get("alwaysThinkingEnabled").getAsBoolean());
    }

    @Test
    void theToolServerConfigNamesThePluginsServerOnItsPort() {
        assertEquals("{\"mcpServers\":{\"eclipse\":{\"type\":\"sse\",\"url\":\"http://127.0.0.1:10123/sse\"}}}",
                TerminalLaunchArgs.mcpConfigJson(10123));
    }
}
