package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.util.List;

import org.junit.jupiter.api.Test;

/**
 * Tests for which launcher a Run As option names, when several carry the same label.
 */
class RunAsToolTest {

    // JDT's and Xtend's launchers both call themselves "JUnit Test"; only JDT's applies to a
    // plain Java project, and the registry happens to hand Xtend's over later.
    private static final List<String> LABELS = List.of("Java Application", "JUnit Test", "Ant Build", "JUnit Test");
    private static final List<String> IDS = List.of("jdt.java", "jdt.junit", "ant.build", "xtend.junit");

    @Test
    void ofTwoLaunchersWithTheSameLabelTheOneThatAppliesIsMeant() {
        assertEquals(1, RunAsTool.choose(LABELS, IDS, List.of(true, true, false, false), "JUnit Test"));
        assertEquals(3, RunAsTool.choose(LABELS, IDS, List.of(true, false, false, true), "junit test"));
    }

    @Test
    void whenNoneOfThatLabelAppliesOneIsStillNamedSoTheRefusalCanSayWhy() {
        assertEquals(3, RunAsTool.choose(LABELS, IDS, List.of(true, false, false, false), "JUnit Test"));
    }

    @Test
    void anIdNamesExactlyItsOwnLauncherWhetherOrNotItApplies() {
        assertEquals(3, RunAsTool.choose(LABELS, IDS, List.of(true, true, false, false), "xtend.junit"));
        assertEquals(1, RunAsTool.choose(LABELS, IDS, List.of(true, true, false, false), "jdt.junit"));
    }

    @Test
    void aLabelNothingCarriesNamesNoLauncher() {
        assertEquals(-1, RunAsTool.choose(LABELS, IDS, List.of(true, true, true, true), "Gradle Test"));
    }
}
