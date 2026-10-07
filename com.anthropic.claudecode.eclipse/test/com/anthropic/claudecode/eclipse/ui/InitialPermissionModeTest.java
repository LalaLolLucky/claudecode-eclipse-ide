package com.anthropic.claudecode.eclipse.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.util.List;

import org.junit.jupiter.api.Test;

/**
 * Tests for the "Initial Permission Mode" preference: what the page offers, and the mode a
 * stored value starts a conversation in.
 */
class InitialPermissionModeTest {

    @Test
    void thePageOffersUnsetThenTheModesWithBypassLast() {
        assertEquals(List.of("", "default", "manual", "acceptEdits", "plan", "bypassPermissions"),
                InitialPermissionMode.choices(true));
    }

    @Test
    void bypassIsNotOfferedWhileThatModeIsNotAllowed() {
        assertEquals(List.of("", "default", "manual", "acceptEdits", "plan"),
                InitialPermissionMode.choices(false));
    }

    @Test
    void unsetStartsAConversationAsItWouldWithoutThePreference() {
        assertEquals("", InitialPermissionMode.resolve("", true));
        assertEquals("", InitialPermissionMode.resolve(null, true));
    }

    @Test
    void aChosenModeIsTheModeStartedIn() {
        assertEquals("default", InitialPermissionMode.resolve("default", false));
        assertEquals("acceptEdits", InitialPermissionMode.resolve("acceptEdits", false));
        assertEquals("plan", InitialPermissionMode.resolve("plan", false));
    }

    @Test
    void manualIsAnotherNameForDefault() {
        // "default" is the spelling every CLI takes; a CLI before 2.1.291 does not know "manual".
        assertEquals("default", InitialPermissionMode.resolve("manual", false));
    }

    @Test
    void bypassIsStartedInOnlyWhileThatModeIsAllowed() {
        assertEquals("bypassPermissions", InitialPermissionMode.resolve("bypassPermissions", true));
        assertEquals("", InitialPermissionMode.resolve("bypassPermissions", false));
    }

    @Test
    void aValueThePageDoesNotOfferCountsAsUnset() {
        assertEquals("", InitialPermissionMode.resolve("yolo", true));
        assertEquals("", InitialPermissionMode.resolve("--dangerously-skip-permissions", true));
    }
}
