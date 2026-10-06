package com.anthropic.claudecode.eclipse.ui.handlers;

import org.eclipse.core.commands.AbstractHandler;
import org.eclipse.core.commands.ExecutionEvent;
import org.eclipse.core.commands.ExecutionException;
import org.eclipse.ui.IViewPart;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.handlers.HandlerUtil;

import com.anthropic.claudecode.eclipse.Activator;
import com.anthropic.claudecode.eclipse.ui.ClaudeCliView;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;

public class RestartServerHandler extends AbstractHandler {

    @Override
    public Object execute(ExecutionEvent event) throws ExecutionException {
        try {
            Activator activator = Activator.getDefault();
            int portBefore = servingPort(activator);
            activator.restart();
            int portAfter = servingPort(activator);

            IWorkbenchPage page = HandlerUtil.getActiveWorkbenchWindow(event).getActivePage();
            if (page != null) {
                // A restart comes back on the port it had whenever it can, and a Claude
                // Terminal session then goes on as it was. When it could not, every open
                // session is still pointed at the old port — for the IDE link and for the
                // plug-in's tools alike — and is started again, as the preference page does
                // when a new port range moves the server.
                if (portMoved(portBefore, portAfter)
                        && page.findView(ClaudeCliView.VIEW_ID) instanceof ClaudeCliView terminal) {
                    ClaudeCodeView.debug("[restart] the server moved from port " + portBefore + " to "
                            + portAfter + "; restarting the Claude Terminal sessions");
                    terminal.restartAllSessions();
                }
                IViewPart viewPart = page.findView(ClaudeCodeView.VIEW_ID);
                if (viewPart instanceof ClaudeCodeView view) {
                    view.restartClaude();
                }
            }
        } catch (Exception e) {
            Activator.logError("Failed to restart server", e);
        }
        return null;
    }

    private static int servingPort(Activator activator) {
        return activator.isServerRunning() ? activator.getHttpSseServer().getPort() : 0;
    }

    /** Whether a server that was serving before and is serving now does so on another port. */
    static boolean portMoved(int before, int after) {
        return before > 0 && after > 0 && before != after;
    }
}
