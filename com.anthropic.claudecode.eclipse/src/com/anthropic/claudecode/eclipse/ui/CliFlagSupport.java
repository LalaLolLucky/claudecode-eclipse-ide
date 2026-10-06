package com.anthropic.claudecode.eclipse.ui;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Collection;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Predicate;
import java.util.function.Supplier;

import com.anthropic.claudecode.eclipse.NativeCore;

/**
 * Which command-line flags the installed CLI knows.
 *
 * <p>A flag the CLI does not know ends it at startup ("unknown option"), so anything that
 * came in with a later version is asked about before it is passed. The answer is read from
 * the program itself — its flags are plain text in it — never taken from a version number,
 * for the reason {@link CliModelSupport} gives.
 *
 * <p>Asked twice over. The native core has this check already (it gates the Claude Code
 * view's own launch flags) and is asked first; this class then reads the program itself for
 * whatever the core did not confirm. That covers a core older than the question, which has
 * no such entry point, and an install the two locate differently. Either one finding the
 * flag is enough, because both hold to the same rule: the flag has to be in the program that
 * will run, as an option of its own — {@code --remote-control} is not found in
 * {@code --remote-control-session-name-prefix}.
 */
final class CliFlagSupport {

    private static final int CHUNK = 4 * 1024 * 1024;

    /** What reading a program found, per program (path, size, modified) and flags asked. */
    private static final Map<String, Set<String>> READ = new ConcurrentHashMap<>();

    private CliFlagSupport() {
    }

    /**
     * Which of {@code flags} the CLI behind {@code claudeCmd} knows. Reads through the
     * program the first time it is asked about one (about a second); later calls only
     * stat it. Never throws: a program that cannot be found or read knows nothing.
     */
    static Predicate<String> knownTo(String claudeCmd, Collection<String> flags) {
        Set<String> known = knownTo(flags, flag -> NativeCore.cliSupportsFlag(claudeCmd, flag),
                () -> readFromProgram(claudeCmd, flags));
        ClaudeCodeView.debug("[cli-flags] " + claudeCmd + " knows " + new TreeSet<>(known) + " of "
                + new TreeSet<>(flags));
        return known::contains;
    }

    /**
     * The rule between the two checks, apart from where their answers come from.
     *
     * @param core the native core's answer for one flag; a core older than the question
     *             throws {@link LinkageError}
     * @param read what reading the program finds, null when it cannot be found or read;
     *             only asked for when the core left a flag unconfirmed
     */
    static Set<String> knownTo(Collection<String> flags, Predicate<String> core, Supplier<Set<String>> read) {
        Set<String> known = new HashSet<>();
        try {
            for (String flag : flags) {
                if (core.test(flag)) known.add(flag);
            }
        } catch (LinkageError olderCore) {
            // A native library from before cliSupportsFlag: the read below answers alone.
        } catch (RuntimeException e) {
            ClaudeCodeView.debug("[cli-flags] the native core could not answer: " + e);
        }
        if (known.size() < flags.size()) {
            Set<String> found = read.get();
            if (found != null) known.addAll(found);
        }
        return known;
    }

    /** What reading the program found, or null when it could not be found or read. */
    private static Set<String> readFromProgram(String claudeCmd, Collection<String> flags) {
        try {
            Path program = CliModelSupport.locateBinary(claudeCmd);
            if (program == null) return null;
            String key = program + "|" + Files.size(program) + "|"
                    + Files.getLastModifiedTime(program).toMillis() + "|" + new TreeSet<>(flags);
            Set<String> found = READ.get(key);
            if (found == null) {
                try (InputStream in = Files.newInputStream(program)) {
                    found = scan(in, flags, CHUNK);
                }
                // One CLI at a time in practice; a second entry means it was updated.
                if (READ.size() > 8) READ.clear();
                READ.put(key, found);
            }
            return found;
        } catch (IOException | RuntimeException e) {
            ClaudeCodeView.debug("[cli-flags] could not read the program behind " + claudeCmd + ": " + e);
            return null;
        }
    }

    /**
     * The flags of {@code flags} that occur in {@code program}. A flag counts only where
     * the byte after it cannot continue it into a longer option name, so
     * {@code --remote-control} is not found in {@code --remote-control-session-name-prefix}.
     * Every flag must start with {@code --}.
     *
     * @param chunk how much is read at a time; a flag lying across two reads is still found
     */
    static Set<String> scan(InputStream program, Collection<String> flags, int chunk) throws IOException {
        byte[][] needles = new byte[flags.size()][];
        String[] names = flags.toArray(new String[0]);
        int longest = 0;
        for (int i = 0; i < names.length; i++) {
            needles[i] = names[i].getBytes(StandardCharsets.US_ASCII);
            longest = Math.max(longest, needles[i].length);
        }

        Set<String> found = new TreeSet<>();
        byte[] buf = new byte[chunk + longest];
        int carry = 0;
        while (found.size() < names.length) {
            int n = program.read(buf, carry, chunk);
            boolean atEnd = n < 0;
            int end = carry + Math.max(n, 0);
            collect(buf, end, atEnd, needles, names, found);
            if (atEnd) break;
            // Keep the tail: a flag that ends exactly at the boundary is judged once the
            // byte after it has been read, and one lying across it is seen whole.
            carry = Math.min(longest, end);
            System.arraycopy(buf, end - carry, buf, 0, carry);
        }
        return found;
    }

    private static void collect(byte[] buf, int end, boolean atEnd, byte[][] needles, String[] names,
            Set<String> found) {
        for (int i = 0; i + 1 < end; i++) {
            if (buf[i] != '-' || buf[i + 1] != '-') continue;
            needle:
            for (int f = 0; f < needles.length; f++) {
                byte[] needle = needles[f];
                int after = i + needle.length;
                if (after > end || found.contains(names[f])) continue;
                for (int k = 2; k < needle.length; k++) {
                    if (buf[i + k] != needle[k]) continue needle;
                }
                if (after == end) {
                    if (!atEnd) continue;   // the next read brings the byte that decides
                } else if (continuesAnOptionName(buf[after])) {
                    continue;
                }
                found.add(names[f]);
            }
        }
    }

    private static boolean continuesAnOptionName(byte b) {
        return b == '-' || (b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z') || (b >= '0' && b <= '9');
    }
}
