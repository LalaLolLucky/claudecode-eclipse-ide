package com.anthropic.claudecode.eclipse.ui.handlers;

import org.eclipse.core.commands.AbstractHandler;
import org.eclipse.core.commands.ExecutionEvent;

import com.anthropic.claudecode.eclipse.ui.ClaudeGuiView;

/**
 * Toggles the find bar for the active Claude Code conversation tab.
 *
 * <p>Bound to Ctrl+F by default (rebindable under Preferences &gt; Keys), scoped to the
 * {@code contexts.guiViewFocus} context — active only while the view's browser control has
 * keyboard focus, so the binding never shadows Ctrl+F elsewhere in the IDE (text editors'
 * own Find/Replace included).
 */
public class FindInConversationHandler extends AbstractHandler {

    @Override
    public Object execute(ExecutionEvent event) {
        ClaudeGuiView.toggleFindInConversation(eventTime(event));
        return null;
    }

    /**
     * The OS-level timestamp of the key event that triggered this command, or -1 if
     * unavailable (programmatic execution, no underlying SWT event). Identical across
     * however many times Eclipse redelivers ONE physical keypress to this handler, and
     * distinct across genuinely separate presses — used by {@link ClaudeGuiView} to dedup
     * a confirmed multi-dispatch without guessing at a time-window threshold.
     */
    private static int eventTime(ExecutionEvent event) {
        Object trigger = event.getTrigger();
        if (trigger instanceof org.eclipse.swt.widgets.Event swtEvent) return swtEvent.time;
        return -1;
    }
}
