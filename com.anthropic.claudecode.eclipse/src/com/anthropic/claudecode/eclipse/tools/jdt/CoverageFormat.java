package com.anthropic.claudecode.eclipse.tools.jdt;

import java.util.ArrayList;
import java.util.List;

import com.google.gson.JsonObject;

/**
 * How the coverage tool writes JaCoCo's figures: a counter as covered, missed, total and a
 * percentage; a file's lines as ranges ("3-5, 9") under their status. Plain ints only, so it
 * is tested without EclEmma.
 */
final class CoverageFormat {

	/** A line's status, as JaCoCo's ICounter numbers it. */
	static final int NOT_COVERED = 1, FULLY_COVERED = 2, PARTLY_COVERED = 3;

	private CoverageFormat() {}

	/** {@code {covered, missed, total, percent}}; no percent when there is nothing to count. */
	static JsonObject counter(int covered, int missed) {
		JsonObject j = new JsonObject();
		int total = covered + missed;
		j.addProperty("covered", covered);
		j.addProperty("missed", missed);
		j.addProperty("total", total);
		if (total > 0) j.addProperty("percent", Math.round(covered * 1000.0 / total) / 10.0);
		return j;
	}

	/** Line numbers in ascending order as ranges: {@code 3-5, 9, 11-12}. */
	static String ranges(int[] lines) {
		StringBuilder out = new StringBuilder();
		int i = 0;
		while (i < lines.length) {
			int start = lines[i], end = start;
			while (i + 1 < lines.length && lines[i + 1] == end + 1) end = lines[++i];
			if (out.length() > 0) out.append(", ");
			out.append(start);
			if (end > start) out.append('-').append(end);
			i++;
		}
		return out.toString();
	}

	/**
	 * A file's lines under their status — {@code covered}, {@code partlyCovered}, {@code missed} —
	 * from {@code status[i]}, the status of line {@code firstLine + i}. Lines without code, and a
	 * status no line has, are left out.
	 */
	static JsonObject lines(int firstLine, int[] status) {
		List<Integer> covered = new ArrayList<>(), partly = new ArrayList<>(), missed = new ArrayList<>();
		for (int i = 0; i < status.length; i++) {
			int line = firstLine + i;
			switch (status[i]) {
				case FULLY_COVERED -> covered.add(line);
				case PARTLY_COVERED -> partly.add(line);
				case NOT_COVERED -> missed.add(line);
				default -> { /* no code on this line */ }
			}
		}
		JsonObject j = new JsonObject();
		put(j, "covered", covered);
		put(j, "partlyCovered", partly);
		put(j, "missed", missed);
		return j;
	}

	private static void put(JsonObject j, String key, List<Integer> lines) {
		if (!lines.isEmpty()) j.addProperty(key, ranges(lines.stream().mapToInt(Integer::intValue).toArray()));
	}
}
