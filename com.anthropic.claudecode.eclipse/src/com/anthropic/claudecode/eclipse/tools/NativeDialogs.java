package com.anthropic.claudecode.eclipse.tools;

import java.util.HashSet;
import java.util.Set;

import com.anthropic.claudecode.eclipse.NativeCore;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * The dialogs the native core finds: the ones that are not SWT widgets of this Eclipse.
 * Thin glue over {@link NativeCore#dialogsList} and {@link NativeCore#dialogsPress}; what
 * counts as a dialog, and whose may be touched, is decided there.
 */
final class NativeDialogs {

    static final String ID_PREFIX = "os-";

    /** A native library built before these calls existed: SWT dialogs only, from then on. */
    private static volatile boolean missing;

    private NativeDialogs() {
    }

    static boolean isNativeId(String id) {
        return id != null && id.startsWith(ID_PREFIX);
    }

    /**
     * What the native core reports, as {@code {"dialogs":[…]}} plus {@code "unavailable"}
     * when the platform refused. Never null; empty when the library cannot answer.
     */
    static JsonObject list(boolean inProcessOnly) {
        JsonObject empty = new JsonObject();
        empty.add("dialogs", new JsonArray());
        if (missing) return empty;
        try {
            JsonObject reply = logged(JsonParser.parseString(NativeCore.dialogsList(inProcessOnly)).getAsJsonObject());
            if (!reply.has("dialogs")) reply.add("dialogs", new JsonArray());
            markForUserOnly(reply.getAsJsonArray("dialogs"));
            return reply;
        } catch (UnsatisfiedLinkError e) {
            missing = true;
            ClaudeCodeView.debug("[eclipseDialog] the native library has no dialog support (rebuild "
                    + "the natives): " + e.getMessage());
            return empty;
        } catch (RuntimeException e) {
            ClaudeCodeView.debug("[eclipseDialog] native list failed: " + e);
            return empty;
        }
    }

    /** {@code {"pressed":…,"dialog":…}} or {@code {"error":…}}. */
    static JsonObject press(String id, String label) {
        try {
            return logged(JsonParser.parseString(NativeCore.dialogsPress(id, label)).getAsJsonObject());
        } catch (UnsatisfiedLinkError e) {
            missing = true;
            return error("This build's native library cannot press dialogs outside Eclipse's own.");
        } catch (RuntimeException e) {
            ClaudeCodeView.debug("[eclipseDialog] native press failed: " + e);
            return error("The native press failed: " + e.getMessage());
        }
    }

    static final boolean WINDOWS =
            System.getProperty("os.name", "").toLowerCase(java.util.Locale.ROOT).startsWith("windows");

    /**
     * Whether a dialog of this Eclipse that the native core reports as well can only be told
     * by its title. Off Windows that is so: the operating system cannot tell an SWT shell
     * from any other window of the process.
     *
     * <p>On Windows it is not. The core counts a window of this process only when it is a
     * Win32 dialog box — and an SWT dialog shell is one (SWT gives it the dialog class,
     * {@code #32770}), so it is reported there too. But there each has a window handle, and
     * the core's id for a window is made from it ({@link #idOfOwnWindow}), so this
     * Eclipse's own shells are matched exactly and the title rule stays off: a native file
     * chooser may well carry the title of the SWT dialog that opened it.
     */
    static final boolean LISTS_SWT_SHELLS_TOO = !WINDOWS;

    /**
     * The id the native core lists a window of this process under: the mirror of
     * {@code make_id} in dialogs.rs, where a Windows window's own token is its handle in
     * hex, and an id carries a token's bytes hex-encoded.
     */
    static String idOfOwnWindow(long pid, long handle) {
        StringBuilder id = new StringBuilder(ID_PREFIX).append(pid).append('-');
        for (char c : Long.toHexString(handle).toCharArray()) {
            id.append(Integer.toHexString(c >> 4)).append(Integer.toHexString(c & 0xf));
        }
        return id.toString();
    }

    /**
     * {@link #withoutThoseIn(JsonArray, JsonArray, boolean, Set)} for the platform this runs
     * on. {@code ownShells} is {@link Dialogs#nativeIdsOf} of this Eclipse's open dialogs.
     */
    static JsonArray withoutThoseIn(JsonArray swt, JsonArray natives, Set<String> ownShells) {
        return withoutThoseIn(swt, natives, LISTS_SWT_SHELLS_TOO, ownShells);
    }

    /** As below, where none of the native dialogs is known to be one of this Eclipse's shells. */
    static JsonArray withoutThoseIn(JsonArray swt, JsonArray natives, boolean listedTwice) {
        return withoutThoseIn(swt, natives, listedTwice, Set.of());
    }

    /**
     * The native dialogs that are not already in {@code swt}. A dialog of this Eclipse can
     * be reported by both sides, and the SWT listing of it is the one kept. Which native
     * entry it is, is known in one of two ways: by its id, when the id is one of
     * {@code ownShells} (the ids this Eclipse's own shells are listed under), or by its
     * title, where titles are all there is to go on ({@code listedTwice}). Another process's
     * dialogs are never the same ones.
     */
    static JsonArray withoutThoseIn(JsonArray swt, JsonArray natives, boolean listedTwice,
            Set<String> ownShells) {
        Set<String> titles = new HashSet<>();
        if (listedTwice) {
            for (JsonElement dialog : swt) titles.add(title(dialog.getAsJsonObject()));
        }
        JsonArray kept = new JsonArray();
        for (JsonElement element : natives) {
            JsonObject dialog = element.getAsJsonObject();
            boolean external = dialog.has("external") && dialog.get("external").getAsBoolean();
            boolean ownShell = dialog.has("id") && ownShells.contains(dialog.get("id").getAsString());
            if (external || !(ownShell || titles.contains(title(dialog)))) kept.add(dialog);
        }
        return kept;
    }

    /**
     * Marks the dialogs that ask about trust as the user's to answer, as {@link Dialogs} does
     * for this Eclipse's own. Of another process only the title and text are known.
     */
    static void markForUserOnly(JsonArray dialogs) {
        for (JsonElement element : dialogs) {
            JsonObject dialog = element.getAsJsonObject();
            String text = dialog.has("text") ? dialog.get("text").getAsString() : null;
            if (Dialogs.asksAboutTrust(title(dialog), text, null)) dialog.addProperty("forUserOnly", true);
        }
    }

    private static String title(JsonObject dialog) {
        return dialog.has("title") ? dialog.get("title").getAsString() : "";
    }

    private static JsonObject logged(JsonObject reply) {
        JsonElement log = reply.remove("log");
        if (log != null && log.isJsonArray()) {
            for (JsonElement line : log.getAsJsonArray()) {
                ClaudeCodeView.debug("[eclipseDialog] " + line.getAsString());
            }
        }
        return reply;
    }

    private static JsonObject error(String message) {
        JsonObject j = new JsonObject();
        j.addProperty("error", message);
        return j;
    }
}
