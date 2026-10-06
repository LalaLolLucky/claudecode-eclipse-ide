package com.anthropic.claudecode.eclipse.ui;

import java.util.HashMap;
import java.util.Map;

import org.eclipse.jface.util.IPropertyChangeListener;
import org.eclipse.jface.util.PropertyChangeEvent;
import org.eclipse.ui.AbstractSourceProvider;
import org.eclipse.ui.ISources;

import com.anthropic.claudecode.eclipse.Activator;
import com.anthropic.claudecode.eclipse.Constants;

/**
 * Publishes the current debug-mode state as a workbench source variable. Both the
 * debug activity's {@code enabledWhen} (which filters the "Claude IDE Server" view
 * and its command from Show View / key bindings) and the "Activate Claude IDE
 * Server" menu item's {@code visibleWhen} key off this single variable.
 *
 * <p>Listens to the preference store and, whenever {@link Constants#PREF_DEBUG_MODE}
 * changes, re-fires the variable. The Show View list, key bindings and the menu item
 * all update without a restart. When debug is switched off we also close the view if
 * it happens to be open.
 *
 * <p>Publishes a second variable the same way: whether the Claude Code view is on
 * offer, which {@link Constants#PREF_TERMINAL_ONLY} ("Exclusively use terminal")
 * switches off. The view's activity and its two menu entries key off it (see
 * {@link TerminalOnlyUi}). It lives in this provider, not one of its own, so that it is
 * defined exactly when {@code debugMode} is: an undefined variable reads as false, and
 * false here would hide the Claude Code view from everyone.
 */
public class DebugModeSourceProvider extends AbstractSourceProvider {

    /** Must match the variable name declared in plugin.xml. */
    public static final String VARIABLE = "com.anthropic.claudecode.eclipse.debugMode";

    /** Must match the variable name declared in plugin.xml. */
    public static final String CODE_VIEW_VARIABLE = "com.anthropic.claudecode.eclipse.codeViewEnabled";

    private final IPropertyChangeListener prefListener = new IPropertyChangeListener() {
        @Override
        public void propertyChange(PropertyChangeEvent event) {
            if (Constants.PREF_DEBUG_MODE.equals(event.getProperty())) {
                boolean debug = DebugModeUi.isDebugEnabled();
                fireSourceChanged(ISources.WORKBENCH, VARIABLE, Boolean.valueOf(debug));
                if (!debug) {
                    DebugModeUi.closeServerViewIfOpen();
                }
            }
            if (Constants.PREF_TERMINAL_ONLY.equals(event.getProperty())) {
                boolean terminalOnly = TerminalOnlyUi.isOn();
                fireSourceChanged(ISources.WORKBENCH, CODE_VIEW_VARIABLE, Boolean.valueOf(!terminalOnly));
                if (terminalOnly) {
                    TerminalOnlyUi.closeCodeViewIfOpen();
                }
            }
        }
    };

    public DebugModeSourceProvider() {
        Activator activator = Activator.getDefault();
        if (activator != null) {
            activator.getPreferenceStore().addPropertyChangeListener(prefListener);
        }
        // From here because this provider is created with the workbench on every start,
        // which the plug-in's early startup is not (the user can switch that off).
        TerminalOnlyUi.install();
    }

    @Override
    public Map<String, Object> getCurrentState() {
        Map<String, Object> state = new HashMap<>(2);
        state.put(VARIABLE, Boolean.valueOf(DebugModeUi.isDebugEnabled()));
        state.put(CODE_VIEW_VARIABLE, Boolean.valueOf(!TerminalOnlyUi.isOn()));
        return state;
    }

    @Override
    public String[] getProvidedSourceNames() {
        return new String[] { VARIABLE, CODE_VIEW_VARIABLE };
    }

    @Override
    public void dispose() {
        Activator activator = Activator.getDefault();
        if (activator != null) {
            activator.getPreferenceStore().removePropertyChangeListener(prefListener);
        }
    }
}
