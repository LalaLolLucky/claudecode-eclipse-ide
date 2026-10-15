package com.anthropic.claudecode.eclipse.tools.jdt;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

import com.google.gson.JsonObject;

/** Tests for how the coverage tool writes JaCoCo's figures: counters and line ranges. */
class CoverageFormatTest {

	@Test
	void aCounterIsItsCoveredMissedAndTotalWithAPercentage() {
		JsonObject lines = CoverageFormat.counter(30, 10);

		assertEquals(30, lines.get("covered").getAsInt());
		assertEquals(10, lines.get("missed").getAsInt());
		assertEquals(40, lines.get("total").getAsInt());
		assertEquals(75.0, lines.get("percent").getAsDouble());
	}

	@Test
	void aPercentageIsRoundedToOneDecimalAndNothingToCountHasNone() {
		assertEquals(33.3, CoverageFormat.counter(1, 2).get("percent").getAsDouble());
		assertEquals(66.7, CoverageFormat.counter(2, 1).get("percent").getAsDouble());
		JsonObject empty = CoverageFormat.counter(0, 0);
		assertEquals(0, empty.get("total").getAsInt());
		assertFalse(empty.has("percent"));
	}

	@Test
	void linesOfOneStatusAreWrittenAsRanges() {
		assertEquals("3-5, 9, 11-12", CoverageFormat.ranges(new int[] { 3, 4, 5, 9, 11, 12 }));
		assertEquals("7", CoverageFormat.ranges(new int[] { 7 }));
		assertEquals("", CoverageFormat.ranges(new int[0]));
	}

	@Test
	void eachLineGoesUnderItsStatusAndLinesWithoutCodeAreLeftOut() {
		// line status per JaCoCo's ICounter: 0 EMPTY, 1 NOT_COVERED, 2 FULLY_COVERED, 3 PARTLY_COVERED
		int first = 10;
		int[] status = { 0, 2, 2, 1, 3, 0, 2, 1, 1 };

		JsonObject lines = CoverageFormat.lines(first, status);

		assertEquals("11-12, 16", lines.get("covered").getAsString());
		assertEquals("14", lines.get("partlyCovered").getAsString());
		assertEquals("13, 17-18", lines.get("missed").getAsString());
	}

	@Test
	void aStatusWithNoLinesIsLeftOut() {
		JsonObject lines = CoverageFormat.lines(1, new int[] { 2, 2 });

		assertEquals("1-2", lines.get("covered").getAsString());
		assertFalse(lines.has("missed"));
		assertFalse(lines.has("partlyCovered"));
		assertTrue(CoverageFormat.lines(1, new int[0]).isEmpty());
	}
}
