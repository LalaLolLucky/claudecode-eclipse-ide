package com.anthropic.claudecode.eclipse.ui;

import java.util.ArrayList;
import java.util.List;

import org.eclipse.jface.preference.IPreferenceStore;

import com.anthropic.claudecode.eclipse.Activator;
import com.anthropic.claudecode.eclipse.Constants;

/**
 * The "Initial Permission Mode" preference: the permission mode a conversation starts in,
 * in the Claude Code view and the Claude Terminal alike.
 *
 * <p>Its values are those of the VS Code extension's setting of the same name
 * ({@code claudeCode.initialPermissionMode}), so a value means here what it means there.
 */
final class InitialPermissionMode {

    /** No mode chosen: the conversation starts as it would without the preference. */
    static final String UNSET = "";

    private static final String DEFAULT = "default";
    /** Another name for {@link #DEFAULT}: what the mode is called on screen. */
    private static final String MANUAL = "manual";
    private static final String BYPASS = "bypassPermissions";

    /** The modes that can be chosen, in the order the page lists them. */
    private static final List<String> MODES = List.of(DEFAULT, MANUAL, "acceptEdits", "plan", BYPASS);

    private InitialPermissionMode() {
    }

    /**
     * What the preference page offers: unset, then the modes — bypass permissions among
     * them only while that mode is allowed at all.
     */
    static List<String> choices(boolean bypassAllowed) {
        List<String> choices = new ArrayList<>();
        choices.add(UNSET);
        for (String mode : MODES) {
            if (bypassAllowed || !BYPASS.equals(mode)) choices.add(mode);
        }
        return choices;
    }

    /**
     * The mode to start a conversation in for a stored value, as the CLI's
     * {@code --permission-mode} takes it; {@link #UNSET} for none.
     *
     * <p>{@code manual} becomes {@code default}, the spelling every CLI knows (one before
     * 2.1.291 refuses {@code manual}). Bypass permissions counts as unset while that mode is
     * not allowed, whatever is stored: the page takes the choice away when the box is
     * unticked, but a store can also be imported or left over. So does anything that is not
     * one of the modes, since the value goes onto a command line.
     */
    static String resolve(String stored, boolean bypassAllowed) {
        if (stored == null || !MODES.contains(stored)) return UNSET;
        if (BYPASS.equals(stored) && !bypassAllowed) return UNSET;
        return MANUAL.equals(stored) ? DEFAULT : stored;
    }

    /** The mode the preferences ask for now. */
    static String current() {
        IPreferenceStore prefs = Activator.getDefault().getPreferenceStore();
        return resolve(prefs.getString(Constants.PREF_INITIAL_PERMISSION_MODE),
                prefs.getBoolean(Constants.PREF_LIVE_AUTO_MODE));
    }
}
