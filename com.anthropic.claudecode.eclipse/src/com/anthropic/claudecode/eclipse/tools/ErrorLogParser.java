package com.anthropic.claudecode.eclipse.tools;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/**
 * Parses Eclipse's workspace log ({@code .metadata/.log}) — the file behind the Error Log
 * view. Plain Java with no Eclipse types, so the format handling can be exercised on its own.
 *
 * <p>The format, as the framework log writes it:
 * <pre>
 * !SESSION 2026-09-29 00:39:08.078 -----------------------------------------------
 * eclipse.buildId=...                  (session header lines, ignored)
 *
 * !ENTRY org.eclipse.ui 4 0 2026-09-29 00:39:21.770
 * !MESSAGE first line of the message
 * continuation lines of the message
 * !STACK 0
 * java.lang.IllegalStateException: ...
 *     at ...
 * !SUBENTRY 1 org.eclipse.jface 2 0 2026-09-29 00:39:21.771
 * !MESSAGE a child status of a multi-status
 * </pre>
 * The number after the plug-in id is the {@code IStatus} severity: 1 info, 2 warning,
 * 4 error, 8 cancel. A line belongs to whatever tag came before it, so messages and stacks
 * may span several lines, blank ones included; only the next {@code !} tag ends them.
 */
public final class ErrorLogParser {

    public static final int INFO = 1;
    public static final int WARNING = 2;
    public static final int ERROR = 4;

    private ErrorLogParser() {}

    /** One {@code !ENTRY} or {@code !SUBENTRY}. */
    public static final class Entry {
        public final int session;   // 0 for entries before any !SESSION line, then 1, 2, …
        public final int depth;     // 0 for !ENTRY, the SUBENTRY depth otherwise
        public final String plugin;
        public final int severity;
        public final String date;
        public final StringBuilder message = new StringBuilder();
        public final StringBuilder stack = new StringBuilder();
        public final List<Entry> children = new ArrayList<>();

        Entry(int session, int depth, String plugin, int severity, String date) {
            this.session = session;
            this.depth = depth;
            this.plugin = plugin;
            this.severity = severity;
            this.date = date;
        }

        /** This entry's severity or its most severe child's, whichever is worse. */
        public int worstSeverity() {
            int worst = severity;
            for (Entry c : children) {
                int cs = c.worstSeverity();
                if (rank(cs) > rank(worst)) worst = cs;
            }
            return worst;
        }

        /** Whether this entry or a child comes from a plug-in whose id contains {@code needle} (lower case). */
        public boolean mentionsPlugin(String needle) {
            if (plugin.toLowerCase(Locale.ROOT).contains(needle)) return true;
            for (Entry c : children) if (c.mentionsPlugin(needle)) return true;
            return false;
        }
    }

    public record Result(List<Entry> entries, int sessions) {
    }

    /** Parses the whole log text into top-level entries, oldest first. */
    public static Result parse(String text) {
        List<Entry> entries = new ArrayList<>();
        int session = 0;
        Entry top = null;       // current !ENTRY
        Entry current = null;   // entry or subentry that message/stack lines attach to
        StringBuilder target = null;

        for (String line : text.split("\r?\n", -1)) {
            if (line.startsWith("!SESSION")) {
                session++;
                top = current = null;
                target = null;
            } else if (line.startsWith("!ENTRY ")) {
                top = current = header(line.substring(7), session, 0);
                if (top != null) entries.add(top);
                target = null;
            } else if (line.startsWith("!SUBENTRY ")) {
                String rest = line.substring(10);
                int sp = rest.indexOf(' ');
                int depth = sp > 0 ? parseInt(rest.substring(0, sp), 1) : 1;
                Entry sub = sp > 0 ? header(rest.substring(sp + 1), session, depth) : null;
                if (sub != null && top != null) {
                    top.children.add(sub);
                    current = sub;
                }
                target = null;
            } else if (line.startsWith("!MESSAGE")) {
                if (current != null) {
                    target = current.message;
                    append(target, line.length() > 9 ? line.substring(9) : "");
                }
            } else if (line.startsWith("!STACK")) {
                target = current != null ? current.stack : null;
            } else if (target != null) {
                append(target, line);
            }
        }
        for (Entry e : entries) trimAll(e);
        return new Result(entries, session);
    }

    /** {@code <plugin> <severity> <code> <date> <time>} → an entry, or null if malformed. */
    private static Entry header(String rest, int session, int depth) {
        String[] parts = rest.trim().split(" +");
        if (parts.length < 2) return null;
        int severity = parseInt(parts[1], INFO);
        String date = parts.length >= 5 ? parts[3] + " " + parts[4] : "";
        return new Entry(session, depth, parts[0], severity, date);
    }

    /** 1 = info/ok/cancel, 2 = warning, 3 = error — so severities compare in order. */
    static int rank(int severity) {
        return severity == ERROR ? 3 : severity == WARNING ? 2 : 1;
    }

    public static String severityName(int severity) {
        return switch (severity) {
            case ERROR -> "ERROR";
            case WARNING -> "WARNING";
            case 8 -> "CANCEL";
            case 0 -> "OK";
            default -> "INFO";
        };
    }

    private static void append(StringBuilder sb, String line) {
        if (sb.length() > 0) sb.append('\n');
        sb.append(line);
    }

    private static void trimAll(Entry e) {
        trimTrailing(e.message);
        trimTrailing(e.stack);
        for (Entry c : e.children) trimAll(c);
    }

    private static void trimTrailing(StringBuilder sb) {
        int end = sb.length();
        while (end > 0 && Character.isWhitespace(sb.charAt(end - 1))) end--;
        sb.setLength(end);
    }

    private static int parseInt(String s, int fallback) {
        try {
            return Integer.parseInt(s.trim());
        } catch (NumberFormatException e) {
            return fallback;
        }
    }
}
