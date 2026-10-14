package com.anthropic.claudecode.eclipse.bridge;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

/** The relay watchdog's retry schedule, one tick at a time. */
public class RelayBackoffTest {

    /** How many ticks in a row the watchdog would skip right now. */
    private static int ticksSkipped(RelayBackoff backoff) {
        int n = 0;
        while (backoff.consumeTick()) {
            n++;
        }
        return n;
    }

    @Test
    public void eachFailureWaitsOneTickLongerThanTheLast() {
        RelayBackoff backoff = new RelayBackoff(20);
        backoff.failed();
        assertEquals(1, ticksSkipped(backoff));
        backoff.failed();
        assertEquals(2, ticksSkipped(backoff));
        backoff.failed();
        assertEquals(3, ticksSkipped(backoff));
    }

    @Test
    public void theWaitStopsGrowingAtTheCap() {
        RelayBackoff backoff = new RelayBackoff(3);
        for (int i = 0; i < 5; i++) {
            backoff.failed();
        }
        assertEquals(3, ticksSkipped(backoff));
    }

    @Test
    public void nothingFailedMeansNoWait() {
        assertEquals(0, ticksSkipped(new RelayBackoff(20)));
    }

    @Test
    public void aDeliberateStartWaitsForNothing() {
        RelayBackoff backoff = new RelayBackoff(20);
        backoff.failed();
        backoff.failed();
        backoff.reset();
        assertEquals(0, ticksSkipped(backoff));
    }

    @Test
    public void aRelayThatStaysUpClearsTheWait() {
        RelayBackoff backoff = new RelayBackoff(20);
        backoff.failed();
        backoff.failed();
        ticksSkipped(backoff);
        backoff.connected();
        backoff.stillRunning();
        backoff.failed();
        assertEquals(1, ticksSkipped(backoff), "the streak should have started over");
    }

    @Test
    public void aRelayThatWasUpAndThenDropsIsReplacedAtOnce() {
        RelayBackoff backoff = new RelayBackoff(20);
        backoff.connected();
        backoff.stillRunning();
        assertFalse(backoff.droppedBeforeSettling());
        assertEquals(0, ticksSkipped(backoff));
    }

    @Test
    public void aRelayStoppedOnPurposeIsNotAFailure() {
        RelayBackoff backoff = new RelayBackoff(20);
        backoff.connected();
        backoff.stopped();
        assertFalse(backoff.droppedBeforeSettling());
        assertEquals(0, ticksSkipped(backoff));
    }

    @Test
    public void aDeliberateStartForgetsARelayThatHadNotSettled() {
        RelayBackoff backoff = new RelayBackoff(20);
        backoff.connected();
        backoff.reset();
        assertFalse(backoff.droppedBeforeSettling());
        assertEquals(0, ticksSkipped(backoff));
    }

    /** The loop in the log: up, gone before the next tick, rebuilt, every 3 seconds. */
    @Test
    public void aRelayThatDropsBeforeTheNextTickCountsAsAFailure() {
        RelayBackoff backoff = new RelayBackoff(20);
        backoff.connected();
        assertTrue(backoff.droppedBeforeSettling());
        assertEquals(1, ticksSkipped(backoff));
        assertFalse(backoff.droppedBeforeSettling(), "one drop is one failure");
    }

    @Test
    public void aRelayThatKeepsDroppingWaitsLongerEachTime() {
        RelayBackoff backoff = new RelayBackoff(20);
        for (int i = 1; i <= 3; i++) {
            backoff.connected();
            assertTrue(backoff.droppedBeforeSettling());
            assertEquals(i, ticksSkipped(backoff));
        }
    }
}
