package com.anthropic.claudecode.eclipse.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

/** Tests for the wording of the one-time notice that conversations were archived. */
class SessionArchiveTest {

    @Test
    void severalSessionsAreToldInThePlural() {
        assertEquals("Claude Code archived 12 sessions with no activity in the last 14 days. "
                + "They are still available under Archived sessions in your session history.",
                SessionArchive.notice(12, 14));
    }

    @Test
    void oneSessionIsToldInTheSingular() {
        assertEquals("Claude Code archived 1 session with no activity in the last 7 days. "
                + "It is still available under Archived sessions in your session history.",
                SessionArchive.notice(1, 7));
    }

    @Test
    void aPeriodOfOneDayIsTheLastDay() {
        assertEquals("Claude Code archived 3 sessions with no activity in the last day. "
                + "They are still available under Archived sessions in your session history.",
                SessionArchive.notice(3, 1));
    }

    @Test
    void withNoPeriodItIsRecentActivity() {
        assertEquals("Claude Code archived 2 sessions with no recent activity. "
                + "They are still available under Archived sessions in your session history.",
                SessionArchive.notice(2, 0));
    }
}
