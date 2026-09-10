package com.anthropic.claudecode.eclipse.ui.handlers;

import org.eclipse.core.commands.AbstractHandler;
import org.eclipse.core.commands.ExecutionEvent;

import com.anthropic.claudecode.eclipse.ui.ClaudeGuiView;

/**
 * Starts or stops voice dictation in the Claude Code composer.
 *
 * <p>Bound to Ctrl+D in {@code plugin.xml}, scoped to the
 * {@code contexts.guiFocused} context so it only applies while the Claude view
 * has focus — unscoped it would shadow Ctrl+D (delete line) in every editor.
 * Being a real command means it shows up under Preferences &gt; General &gt; Keys
 * and can be rebound like anything else.
 *
 * <p>The page owns the gesture state (held vs latched), so this delegates to the
 * same {@code micToggle()} the button uses rather than reimplementing it. That
 * also means the keyboard path can never disagree with the pointer path about
 * whether a take is running.
 */
public class ToggleDictationHandler extends AbstractHandler {

    @Override
    public Object execute(ExecutionEvent event) {
        ClaudeGuiView.toggleDictation();
        return null;
    }
}
