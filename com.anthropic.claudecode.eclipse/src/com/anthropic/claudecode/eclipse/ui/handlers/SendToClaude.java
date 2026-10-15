package com.anthropic.claudecode.eclipse.ui.handlers;

import org.eclipse.core.commands.ExecutionEvent;
import org.eclipse.jface.dialogs.MessageDialog;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.IWorkbenchWindow;
import org.eclipse.ui.handlers.HandlerUtil;

import com.anthropic.claudecode.eclipse.Activator;
import com.anthropic.claudecode.eclipse.ui.ClaudeCliView;
import com.anthropic.claudecode.eclipse.ui.ClaudeGuiView;

/**
 * Where "Send to Claude Terminal" and "Send to Claude Code" put their text: at the prompt of
 * the Claude Terminal tab in front, or in the Claude Code view's composer. Either way it is
 * not sent; it waits there with the view brought forward. Neither view is opened for it: one
 * that is not open is said so, as for the editor's Send Selection.
 */
final class SendToClaude {

    /** The view a handler sends to, as plugin.xml names it after the handler's class. */
    enum Target {
        TERMINAL, CODE;

        static Target of(Object data) {
            return "code".equals(data) ? CODE : TERMINAL;
        }
    }

    private SendToClaude() {}

    /** The folder an {@code @}-mention is written relative to, for the view text goes to; null
     *  when it is not open. */
    static String folderFor(ExecutionEvent event, Target target) {
        if (target == Target.CODE) return ClaudeGuiView.frontFolder();
        ClaudeCliView terminal = openTerminal(event);
        return terminal != null ? terminal.getActiveSessionCwd() : null;
    }

    /** Puts {@code text} at the prompt of the view the handler is for. */
    static void deliver(ExecutionEvent event, String text, Target target) {
        if (text == null || text.isBlank()) return;
        if (target == Target.CODE) {
            IWorkbenchPage page = page(event);
            if (page != null && page.findView(ClaudeGuiView.VIEW_ID) instanceof ClaudeGuiView view) {
                page.activate(view);
                view.addToComposer(text);
            } else {
                IWorkbenchWindow window = HandlerUtil.getActiveWorkbenchWindow(event);
                MessageDialog.openInformation(window != null ? window.getShell() : null,
                        "Claude Code", "No Claude Code view open.");
            }
            return;
        }
        ClaudeCliView terminal = AddFileHandler.findOpenTerminal(event);
        if (terminal == null) return;
        String typed = SendToClaudeText.forTerminal(text);
        // One line gets a space after it, as a mention typed by Send Selection does.
        if (!typed.startsWith("\u001b")) typed += " ";
        if (!terminal.sendTextToActiveSession(typed)) {
            AddFileHandler.showNoTerminalDialog(event);
        } else {
            Activator.log("Sent to the Claude Terminal: " + text.length() + " characters");
        }
    }

    private static ClaudeCliView openTerminal(ExecutionEvent event) {
        IWorkbenchPage page = page(event);
        return page != null && page.findView(ClaudeCliView.VIEW_ID) instanceof ClaudeCliView view ? view : null;
    }

    private static IWorkbenchPage page(ExecutionEvent event) {
        IWorkbenchWindow window = HandlerUtil.getActiveWorkbenchWindow(event);
        return window != null ? window.getActivePage() : null;
    }
}
