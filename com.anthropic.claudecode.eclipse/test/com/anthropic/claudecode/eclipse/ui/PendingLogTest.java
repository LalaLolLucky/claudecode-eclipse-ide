package com.anthropic.claudecode.eclipse.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.util.ArrayList;
import java.util.List;

import org.junit.jupiter.api.Test;

class PendingLogTest {

    @Test
    void linesComeBackInTheOrderTheyCame() {
        PendingLog log = new PendingLog(100, 10_000);
        log.add("[mcp] → list");
        log.add("[cli] ← ok");
        assertEquals("[mcp] → list\n[cli] ← ok\n", log.drainAll());
    }

    @Test
    void takingThemLeavesNone() {
        PendingLog log = new PendingLog(100, 10_000);
        log.add("a");
        log.drainAll();
        assertEquals("", log.drainAll());
        assertEquals("", new PendingLog(100, 10_000).drainAll());
    }

    @Test
    void overTheLineBoundTheOldestGo() {
        PendingLog log = new PendingLog(3, 10_000);
        for (String s : new String[] { "1", "2", "3", "4", "5" }) log.add(s);
        assertEquals("3\n4\n5\n", log.drainAll());
    }

    @Test
    void overTheCharacterBoundTheOldestGo() {
        PendingLog log = new PendingLog(100, 10);
        log.add("aaaa");
        log.add("bbbb");
        log.add("cccc");   // 12 characters kept would be over 10: "aaaa" goes
        assertEquals("bbbb\ncccc\n", log.drainAll());
    }

    @Test
    void aLineLongerThanTheWholeBoundIsKeptAlone() {
        PendingLog log = new PendingLog(100, 10);
        log.add("short");
        log.add("x".repeat(25));
        assertEquals("x".repeat(25) + "\n", log.drainAll());
    }

    @Test
    void aSnapshotLeavesThemKept() {
        PendingLog log = new PendingLog(100, 10_000);
        log.add("a");
        log.add("b");
        assertEquals("a\nb\n", log.snapshot());
        assertEquals("a\nb\n", log.drainAll());
        assertEquals("", log.snapshot());
    }

    @Test
    void clearForgetsEverything() {
        PendingLog log = new PendingLog(100, 10_000);
        log.add("a");
        log.clear();
        assertEquals("", log.drainAll());
    }

    @Test
    void linesFromSeveralThreadsAtOnceAreAllKept() throws Exception {
        PendingLog log = new PendingLog(10_000, 1_000_000);
        List<Thread> threads = new ArrayList<>();
        for (int t = 0; t < 8; t++) {
            int id = t;
            threads.add(new Thread(() -> { for (int i = 0; i < 250; i++) log.add(id + ":" + i); }));
        }
        for (Thread th : threads) th.start();
        for (Thread th : threads) th.join();
        assertEquals(2000, log.drainAll().split("\n").length);
    }
}
