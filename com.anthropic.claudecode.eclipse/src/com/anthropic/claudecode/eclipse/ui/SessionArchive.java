package com.anthropic.claudecode.eclipse.ui;

import java.util.ArrayList;
import java.util.List;

import com.anthropic.claudecode.eclipse.Activator;
import com.anthropic.claudecode.eclipse.Constants;
import com.anthropic.claudecode.eclipse.NativeCore;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * The Claude Code view's side of its archive: which saved conversations the history lists
 * under "Archived sessions". The record and its rules are the core's (archive.rs); this
 * names the file, reads the preference and words the one-time notice.
 */
final class SessionArchive {

    private static final String FILE = "gui-session-archive.json";

    private SessionArchive() {
    }

    /**
     * A history list with the archive applied.
     *
     * @param sessionsJson the rows, each with an {@code archived} flag when {@code available}
     * @param available    false when the native library has no archive (an older build): the
     *                     rows are then the plain list, and the page offers no archive
     * @param archivedNow  the conversations this listing archived for inactivity
     */
    record Listing(String sessionsJson, boolean available, List<String> archivedNow) {
    }

    /**
     * Applies the archive to a folder's history list. Not for the UI thread.
     *
     * @param sweep whether to archive what has been inactive for the preference's period;
     *              without it the rows are only marked
     */
    static Listing apply(String root, String sessionsJson, String openIdsJson, boolean sweep) {
        try {
            int days = sweep
                    ? Activator.getDefault().getPreferenceStore().getInt(Constants.PREF_ARCHIVE_INACTIVE_SESSIONS)
                    : 0;
            JsonObject out = JsonParser.parseString(
                    NativeCore.sessionArchiveApply(storePath(), root, sessionsJson, days, openIdsJson)).getAsJsonObject();
            List<String> archivedNow = new ArrayList<>();
            for (JsonElement id : out.getAsJsonArray("archivedNow")) archivedNow.add(id.getAsString());
            return new Listing(out.getAsJsonArray("sessions").toString(), true, archivedNow);
        } catch (Throwable t) {
            // LinkageError from a native library built before the archive, or an answer
            // that is not one: the history is then exactly what it was without it.
            return new Listing(sessionsJson, false, List.of());
        }
    }

    /** Archives or unarchives conversations by hand. Returns whether it was recorded. */
    static boolean set(String idsJson, boolean archived) {
        try {
            return NativeCore.sessionArchiveSet(storePath(), idsJson, archived);
        } catch (Throwable t) {
            return false;
        }
    }

    private static String storePath() {
        return Activator.getDefault().getStateLocation().append(FILE).toOSString();
    }

    /**
     * What the user is told the first time conversations are archived automatically — the
     * VS Code extension's wording for the same notice.
     *
     * @param count how many were archived
     * @param days  the period they were inactive for, as the preference has it
     */
    static String notice(int count, int days) {
        String sessions = count == 1 ? "1 session" : count + " sessions";
        String inactivity = days == 0 ? "with no recent activity"
                : days == 1 ? "with no activity in the last day"
                : "with no activity in the last " + days + " days";
        return "Claude Code archived " + sessions + " " + inactivity + ". "
                + (count == 1 ? "It is" : "They are")
                + " still available under Archived sessions in your session history.";
    }
}
