package com.anthropic.claudecode.eclipse.ui;

import java.util.ArrayDeque;

/**
 * The Debug-mode lines of the Claude IDE Server view that came while nobody could see them:
 * before the view was first shown, or while it was closed. The view takes them all when it
 * goes live ({@link ClaudeCodeView}). Bounded by lines and by characters, oldest dropped
 * first, so a busy relay ({@code [BRIDGE]} logs every message) cannot make it grow without end.
 */
final class PendingLog {

    private final int maxLines;
    private final int maxChars;
    private final ArrayDeque<String> lines = new ArrayDeque<>();
    private long chars;

    PendingLog(int maxLines, int maxChars) {
        this.maxLines = maxLines;
        this.maxChars = maxChars;
    }

    /** Keeps {@code line}, dropping the oldest lines while over either bound. */
    synchronized void add(String line) {
        lines.addLast(line);
        chars += line.length();
        // The newest line is always kept, even one longer than the whole bound.
        while (lines.size() > 1 && (lines.size() > maxLines || chars > maxChars)) {
            chars -= lines.removeFirst().length();
        }
    }

    /** Every line kept, oldest first, each ended by a newline; and none kept after. */
    synchronized String drainAll() {
        String all = snapshot();
        clear();
        return all;
    }

    /** Every line kept, as {@link #drainAll} gives them, but left kept. */
    synchronized String snapshot() {
        if (lines.isEmpty()) return "";
        StringBuilder sb = new StringBuilder((int) Math.min(Integer.MAX_VALUE - 8, chars + lines.size()));
        for (String line : lines) sb.append(line).append('\n');
        return sb.toString();
    }

    /** Forgets every line kept. */
    synchronized void clear() {
        lines.clear();
        chars = 0;
    }
}
