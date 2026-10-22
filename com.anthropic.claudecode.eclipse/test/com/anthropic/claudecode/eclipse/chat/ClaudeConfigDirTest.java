package com.anthropic.claudecode.eclipse.chat;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.nio.file.Paths;

import org.junit.jupiter.api.Test;

class ClaudeConfigDirTest {

    @Test
    void theFolderCLAUDE_CONFIG_DIRNamesIsTheOneUsed() {
        assertEquals(Paths.get("/data/claude"), ClaudeConfigDir.resolve("/data/claude", "/home/me"));
    }

    @Test
    void withoutOneItIsDotClaudeInTheHomeFolder() {
        assertEquals(Paths.get("/home/me", ".claude"), ClaudeConfigDir.resolve(null, "/home/me"));
    }

    @Test
    void anEmptyValueCountsAsNotSet() {
        assertEquals(Paths.get("/home/me", ".claude"), ClaudeConfigDir.resolve("", "/home/me"));
    }
}
