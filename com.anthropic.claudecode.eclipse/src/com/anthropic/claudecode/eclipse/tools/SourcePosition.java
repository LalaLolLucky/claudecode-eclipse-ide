package com.anthropic.claudecode.eclipse.tools;

/**
 * A position in source text, as people give it (line and column, both from 1) and as indexes
 * want it (a character offset), and the identifier at a position. Line endings may be
 * {@code \n}, {@code \r\n} or {@code \r}.
 */
public final class SourcePosition {

	private SourcePosition() {}

	/** The offset of {@code line}:{@code column}, or -1 when the text has no such place. A
	 *  column one past the end of a line is that line's end. */
	public static int offset(String text, int line, int column) {
		if (text == null || line < 1 || column < 1) return -1;
		int start = 0;
		for (int l = 1; l < line; l++) {
			int next = nextLineStart(text, start);
			if (next < 0) return -1;
			start = next;
		}
		int end = lineEnd(text, start);
		int at = start + column - 1;
		return at <= end ? at : -1;
	}

	/** The line and column (from 1) of {@code offset}. */
	public static int[] lineAndColumn(String text, int offset) {
		int line = 1, start = 0;
		while (true) {
			int next = nextLineStart(text, start);
			if (next < 0 || next > offset) break;
			start = next;
			line++;
		}
		return new int[] { line, offset - start + 1 };
	}

	/**
	 * The start and end of the identifier at {@code offset} or ending just before it (where a
	 * caret after a word stands), or null when there is none there.
	 */
	public static int[] identifierAt(String text, int offset) {
		if (text == null || offset < 0 || offset > text.length()) return null;
		int at = offset;
		if (at == text.length() || !part(text.charAt(at))) {
			if (at > 0 && part(text.charAt(at - 1))) at--;
			else return null;
		}
		int start = at, end = at;
		while (start > 0 && part(text.charAt(start - 1))) start--;
		while (end < text.length() && part(text.charAt(end))) end++;
		return Character.isDigit(text.charAt(start)) ? null : new int[] { start, end };
	}

	private static boolean part(char c) {
		return Character.isLetterOrDigit(c) || c == '_' || c == '$';
	}

	/** Where the line after the one starting at {@code start} starts, or -1 after the last one. */
	private static int nextLineStart(String text, int start) {
		int end = lineEnd(text, start);
		if (end >= text.length()) return -1;
		return text.startsWith("\r\n", end) ? end + 2 : end + 1;
	}

	private static int lineEnd(String text, int start) {
		int i = start;
		while (i < text.length() && text.charAt(i) != '\n' && text.charAt(i) != '\r') i++;
		return i;
	}
}
