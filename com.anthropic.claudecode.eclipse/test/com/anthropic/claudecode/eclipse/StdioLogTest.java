package com.anthropic.claudecode.eclipse;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTimeoutPreemptively;

import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

import org.junit.jupiter.api.Test;

/** Lines for stdout/stderr that never hold up the code that writes them. */
public class StdioLogTest {

    @Test
    public void linesArriveInOrderOnTheStreamTheyWereWrittenTo() throws Exception {
        List<String> seen = new ArrayList<>();
        CountDownLatch all = new CountDownLatch(3);
        StdioLog log = new StdioLog((toErr, line) -> {
            synchronized (seen) {
                seen.add((toErr ? "err:" : "out:") + line);
            }
            all.countDown();
        }, 16);
        log.line(false, "a");
        log.line(true, "b");
        log.line(false, "c");
        all.await(10, TimeUnit.SECONDS);
        synchronized (seen) {
            assertEquals(List.of("out:a", "err:b", "out:c"), seen);
        }
    }

    /** A pipe nobody reads: the write never returns. The caller must not notice. */
    @Test
    public void aStuckStreamNeverHoldsUpTheCaller() {
        CountDownLatch never = new CountDownLatch(1);
        StdioLog log = new StdioLog((toErr, line) -> {
            try {
                never.await();
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }, 4);
        try {
            assertTimeoutPreemptively(Duration.ofSeconds(2), () -> {
                for (int i = 0; i < 10_000; i++) {
                    log.line(true, "line " + i);
                }
            });
        } finally {
            never.countDown();
        }
    }

    /** A sink that throws on one line still gets the next. */
    @Test
    public void aLineThatFailsDoesNotStopTheNext() throws Exception {
        CountDownLatch second = new CountDownLatch(1);
        StdioLog log = new StdioLog((toErr, line) -> {
            if (line.equals("first")) {
                throw new IllegalStateException("broken stream");
            }
            second.countDown();
        }, 16);
        log.line(true, "first");
        log.line(true, "second");
        assertEquals(true, second.await(10, TimeUnit.SECONDS));
    }
}
