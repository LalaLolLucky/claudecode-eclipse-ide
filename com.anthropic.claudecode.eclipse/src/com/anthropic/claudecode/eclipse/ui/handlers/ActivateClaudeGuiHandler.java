package com.anthropic.claudecode.eclipse.ui.handlers;

import org.eclipse.core.commands.AbstractHandler;
import org.eclipse.core.commands.ExecutionEvent;
import org.eclipse.core.commands.ExecutionException;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.handlers.HandlerUtil;

import com.anthropic.claudecode.eclipse.Activator;
import com.anthropic.claudecode.eclipse.ui.ClaudeGuiView;
import com.anthropic.claudecode.eclipse.ui.TerminalOnlyUi;

/** Shows/focuses the Claude Code view — always reveals, never hides (mirror of
 *  {@link ActivateClaudeCliHandler}). While the terminal is used exclusively the command
 *  is not on offer; reached anyway (a key the user bound to it), it opens the Claude
 *  Terminal instead. */
public class ActivateClaudeGuiHandler extends AbstractHandler {

    @Override
    public Object execute(ExecutionEvent event) throws ExecutionException {
        try {
            IWorkbenchPage page = HandlerUtil.getActiveWorkbenchWindow(event).getActivePage();
            if (page == null) return null;

            // Ensure the MCP server is running before showing the view.
            if (!Activator.getDefault().isServerRunning()) {
                Activator.getDefault().initialize();
            }
            if (TerminalOnlyUi.isOn()) {
                TerminalOnlyUi.openTerminal(page);
                return null;
            }
            page.showView(ClaudeGuiView.VIEW_ID);
        } catch (Exception e) {
            Activator.logError("Failed to activate Claude Code view", e);
        }
        return null;
    }
}
