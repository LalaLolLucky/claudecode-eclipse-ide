package com.anthropic.claudecode.eclipse.ui.handlers;

import java.nio.file.Path;
import java.nio.file.Paths;

import org.eclipse.core.resources.IMarker;

/**
 * What "Send to Claude Terminal" and "Send to Claude Code" put at Claude's prompt from the
 * Problems and Console views: a problem as its file at its line ({@code @path#L12}, the
 * mention the CLI reads) followed by its severity and message, or the console's text.
 */
final class SendToClaudeText {

    /** What a terminal sends around pasted text while the program in it asks for that
     *  (bracketed paste): the CLI then takes the line breaks inside as part of the text. */
    private static final String PASTE_START = "\u001b[200~";
    private static final String PASTE_END = "\u001b[201~";

    private SendToClaudeText() {}

    /** One problem on one line: {@code @path#L12 Error: message}. The file or the line is
     *  left out when the problem has none. */
    static String problem(String mentionPath, int line, int severity, String message) {
        StringBuilder out = new StringBuilder();
        if (mentionPath != null && !mentionPath.isEmpty()) {
            out.append('@').append(mentionPath);
            if (line > 0) out.append("#L").append(line);
            out.append(' ');
        }
        out.append(severityName(severity)).append(": ");
        out.append(message == null ? "" : message.strip().replaceAll("\\s*\\R\\s*", " "));
        return out.toString();
    }

    static String severityName(int severity) {
        switch (severity) {
            case IMarker.SEVERITY_ERROR:
                return "Error";
            case IMarker.SEVERITY_WARNING:
                return "Warning";
            default:
                return "Info";
        }
    }

    /**
     * A file as an {@code @}-mention names it: relative to {@code folder} when it lies under it,
     * otherwise in full, with forward slashes, and in quotes when it holds a space.
     */
    static String mentionPath(String absPath, String folder) {
        String result = absPath;
        if (folder != null && !folder.isEmpty()) {
            try {
                Path rel = Paths.get(folder).relativize(Paths.get(absPath));
                if (!rel.toString().isEmpty() && !rel.startsWith("..")) result = rel.toString();
            } catch (IllegalArgumentException differentRoot) {
                // another drive: kept in full
            }
        }
        result = result.replace('\\', '/');
        return result.contains(" ") ? '"' + result + '"' : result;
    }

    /**
     * The console's text to send: what is selected, or the whole line the caret is on when
     * nothing is. Null when that is empty.
     */
    static String consoleText(String document, int offset, int length) {
        if (document == null || offset < 0 || offset > document.length()) return null;
        String text;
        if (length > 0) {
            text = document.substring(offset, Math.min(document.length(), offset + length));
        } else {
            int start = document.lastIndexOf('\n', offset - 1) + 1;
            int end = document.indexOf('\n', offset);
            if (end < 0) end = document.length();
            text = document.substring(start, end);
        }
        text = text.replaceAll("\\R+$", "");
        if (text.endsWith("\r")) text = text.substring(0, text.length() - 1);
        return text.isBlank() ? null : text;
    }

    /** Text for the Claude Terminal's prompt. One line goes as it is; several go as one paste,
     *  or each line break would be an Enter and send the prompt half written. */
    static String forTerminal(String text) {
        String trimmed = text.replaceAll("\\R+$", "");
        if (!trimmed.contains("\n") && !trimmed.contains("\r")) return trimmed;
        return PASTE_START + trimmed.replaceAll("\\R", "\r") + PASTE_END;
    }
}
