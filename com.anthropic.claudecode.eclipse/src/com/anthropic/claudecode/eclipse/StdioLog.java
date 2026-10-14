package com.anthropic.claudecode.eclipse;

import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.BlockingQueue;

/**
 * Lines for this process's stdout and stderr that never hold up the code writing them.
 * <p>
 * Eclipse's stdout and stderr are whatever started Eclipse handed it. A launcher that
 * starts Eclipse as a child and never reads its output (PDE Tools' "Launch New
 * Workspace" does) hands it pipes that fill up, after which a plain {@code println}
 * waits for good — on whatever thread made it, lock held or not. So lines are queued
 * for one daemon thread of their own and dropped when the queue is full: only that
 * thread can ever wait on the stream. The native core does the same for its lines
 * ({@code stdio_log.rs}).
 */
public final class StdioLog {

    /** Where the writer thread puts each line. */
    interface Sink {
        void write(boolean toErr, String line) throws Exception;
    }

    private static final int QUEUE_LINES = 1024;

    private static final StdioLog SHARED = new StdioLog(
            // System.out/err read per line, as the println it replaces did.
            (toErr, line) -> (toErr ? System.err : System.out).println(line), QUEUE_LINES);

    private record Line(boolean toErr, String text) {}

    private final Sink sink;
    private final BlockingQueue<Line> queue;

    StdioLog(Sink sink, int queueLines) {
        this.sink = sink;
        this.queue = new ArrayBlockingQueue<>(queueLines);
        Thread writer = new Thread(this::drain, "claude-stdio-log");
        writer.setDaemon(true);
        writer.start();
    }

    /** Queues the line, or drops it when the queue is full. Never waits, never throws. */
    void line(boolean toErr, String text) {
        queue.offer(new Line(toErr, text));
    }

    private void drain() {
        while (true) {
            Line line;
            try {
                line = queue.take();
            } catch (InterruptedException e) {
                return;
            }
            try {
                sink.write(line.toErr(), line.text());
            } catch (Exception e) {
                // A stream that failed on this line: drop it and carry on with the next.
            }
        }
    }

    /** {@code System.out.println(line)}, without the wait. */
    public static void out(String line) {
        SHARED.line(false, line);
    }

    /** {@code System.err.println(line)}, without the wait. */
    public static void err(String line) {
        SHARED.line(true, line);
    }
}
