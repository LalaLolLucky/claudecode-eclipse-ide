package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

import org.junit.jupiter.api.Test;

/** Tests for reading a position in source text: line and column to an offset, and the word there. */
class SourcePositionTest {

	private static final String TEXT = "int add(int a, int b);\r\n\r\nint main() {\r\n  return add(1, 2);\r\n}\n";

	@Test
	void aLineAndAColumnBecomeAnOffsetWhateverTheLineEndings() {
		assertEquals(0, SourcePosition.offset(TEXT, 1, 1));
		assertEquals(4, SourcePosition.offset(TEXT, 1, 5));
		assertEquals(24, SourcePosition.offset(TEXT, 2, 1));
		assertEquals(40, SourcePosition.offset(TEXT, 4, 1));
		assertEquals(49, SourcePosition.offset(TEXT, 4, 10));
		assertEquals(TEXT.indexOf('}'), SourcePosition.offset(TEXT, 5, 1));
	}

	@Test
	void aPositionOutsideTheTextHasNoOffset() {
		assertEquals(-1, SourcePosition.offset(TEXT, 0, 1));
		assertEquals(-1, SourcePosition.offset(TEXT, 9, 1));
		assertEquals(-1, SourcePosition.offset(TEXT, 1, 0));
		assertEquals(-1, SourcePosition.offset(TEXT, 1, 40));
	}

	@Test
	void theIdentifierAroundAnOffsetIsItsStartAndEnd() {
		int at = TEXT.indexOf("add(1");
		assertArrayEquals(new int[] { at, at + 3 }, SourcePosition.identifierAt(TEXT, at));
		assertArrayEquals(new int[] { at, at + 3 }, SourcePosition.identifierAt(TEXT, at + 2));
		assertArrayEquals(new int[] { at, at + 3 }, SourcePosition.identifierAt(TEXT, at + 3));   // just after it
		int main = TEXT.indexOf("main");
		assertArrayEquals(new int[] { main, main + 4 }, SourcePosition.identifierAt(TEXT, main + 1));
	}

	@Test
	void whereThereIsNoIdentifierThereIsNone() {
		assertNull(SourcePosition.identifierAt(TEXT, TEXT.indexOf("(1, 2)") + 3));
		assertNull(SourcePosition.identifierAt(TEXT, -1));
		assertNull(SourcePosition.identifierAt("", 0));
	}

	@Test
	void anOffsetHasALineAndAColumn() {
		assertArrayEquals(new int[] { 4, 10 }, SourcePosition.lineAndColumn(TEXT, 49));
		assertArrayEquals(new int[] { 1, 1 }, SourcePosition.lineAndColumn(TEXT, 0));
		assertArrayEquals(new int[] { 2, 1 }, SourcePosition.lineAndColumn(TEXT, 24));
	}
}
