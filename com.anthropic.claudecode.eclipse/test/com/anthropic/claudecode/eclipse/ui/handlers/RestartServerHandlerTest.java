package com.anthropic.claudecode.eclipse.ui.handlers;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

/**
 * Tests for when a server restart has to take the Claude Terminal's sessions with it.
 */
class RestartServerHandlerTest {

    @Test
    void aServerBackOnTheSamePortLeavesTheSessionsAlone() {
        assertFalse(RestartServerHandler.portMoved(10123, 10123));
    }

    @Test
    void aServerOnAnotherPortHasLeftTheSessionsBehind() {
        assertTrue(RestartServerHandler.portMoved(10123, 10124));
    }

    @Test
    void aServerThatWasNotRunningBeforeOrIsNotNowMovedNothing() {
        assertFalse(RestartServerHandler.portMoved(0, 10123));
        assertFalse(RestartServerHandler.portMoved(10123, 0));
    }
}
