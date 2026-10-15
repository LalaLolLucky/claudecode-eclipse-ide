package com.anthropic.claudecode.eclipse.ui.handlers;

import org.eclipse.core.commands.AbstractHandler;
import org.eclipse.core.commands.ExecutionEvent;
import org.eclipse.core.runtime.IConfigurationElement;
import org.eclipse.core.runtime.IExecutableExtension;
import org.eclipse.jface.text.IDocument;
import org.eclipse.jface.text.ITextSelection;
import org.eclipse.ui.IWorkbenchPart;
import org.eclipse.ui.console.TextConsolePage;
import org.eclipse.ui.console.TextConsoleViewer;
import org.eclipse.ui.handlers.HandlerUtil;
import org.eclipse.ui.part.PageBookView;

import com.anthropic.claudecode.eclipse.Activator;

/**
 * "Send to Claude Terminal" / "Send to Claude Code" in the Console view: the selected text, or
 * the line the caret is on when nothing is selected. The menu is the console page's own
 * ({@code <console type>.#ContextMenu}, see TextConsolePage#createControl). plugin.xml names the
 * target after the class: {@code :terminal} or {@code :code}.
 */
public class SendConsoleHandler extends AbstractHandler implements IExecutableExtension {

    private SendToClaude.Target target = SendToClaude.Target.TERMINAL;

    @Override
    public void setInitializationData(IConfigurationElement config, String propertyName, Object data) {
        target = SendToClaude.Target.of(data);
    }

    @Override
    public Object execute(ExecutionEvent event) {
        try {
            IWorkbenchPart part = HandlerUtil.getActivePart(event);
            if (!(part instanceof PageBookView book) || !(book.getCurrentPage() instanceof TextConsolePage page)) return null;
            TextConsoleViewer viewer = page.getViewer();
            IDocument document = viewer != null ? viewer.getDocument() : null;
            if (document == null || !(viewer.getSelection() instanceof ITextSelection selection)) return null;
            SendToClaude.deliver(event,
                    SendToClaudeText.consoleText(document.get(), selection.getOffset(), selection.getLength()), target);
        } catch (RuntimeException e) {
            Activator.logError("Failed to send console text to Claude", e);
        }
        return null;
    }
}
