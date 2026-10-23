package com.anthropic.claudecode.eclipse.chat;

import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Locale;

/**
 * The folder the Claude Code CLI keeps its files in, which is where its transcripts are written and
 * so where they are read: the one {@code CLAUDE_CONFIG_DIR} names when it names one, and
 * {@code ~/.claude} otherwise. The native core resolves it the same way ({@code session::config_dir}).
 */
public final class ClaudeConfigDir {

    private ClaudeConfigDir() {}

    /** The folder, for this process's environment. */
    public static Path path() {
        return resolve(System.getenv("CLAUDE_CONFIG_DIR"), homeDir());
    }

    /** The folder transcripts are kept under: one folder for each working directory. */
    public static Path projects() {
        return path().resolve("projects");
    }

    /** {@code configDir} when it names a folder (an empty value counts as unset), else {@code <home>/.claude}. */
    static Path resolve(String configDir, String home) {
        if (configDir != null && !configDir.isEmpty()) return Paths.get(configDir);
        return Paths.get(home, ".claude");
    }

    private static String homeDir() {
        boolean windows = System.getProperty("os.name", "").toLowerCase(Locale.ROOT).contains("win");
        String home = System.getenv(windows ? "USERPROFILE" : "HOME");
        return home == null || home.isEmpty() ? System.getProperty("user.home") : home;
    }
}
