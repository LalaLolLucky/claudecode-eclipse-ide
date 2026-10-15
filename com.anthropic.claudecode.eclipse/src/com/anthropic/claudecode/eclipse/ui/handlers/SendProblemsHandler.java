package com.anthropic.claudecode.eclipse.ui.handlers;

import java.util.ArrayList;
import java.util.List;

import org.eclipse.core.commands.AbstractHandler;
import org.eclipse.core.commands.ExecutionEvent;
import org.eclipse.core.resources.IMarker;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.runtime.Adapters;
import org.eclipse.core.runtime.IConfigurationElement;
import org.eclipse.core.runtime.IExecutableExtension;
import org.eclipse.core.runtime.IPath;
import org.eclipse.jface.viewers.ISelection;
import org.eclipse.jface.viewers.IStructuredSelection;
import org.eclipse.ui.handlers.HandlerUtil;

import com.anthropic.claudecode.eclipse.Activator;

/**
 * "Send to Claude Terminal" / "Send to Claude Code" in the Problems view: each selected problem
 * on a line of its own, as its file at its line with its severity and message. The entries of
 * that view adapt to their {@link IMarker} (MarkerEntry#getAdapter, in 4.26 as in 4.37+).
 * plugin.xml names the target after the class: {@code :terminal} or {@code :code}.
 */
public class SendProblemsHandler extends AbstractHandler implements IExecutableExtension {

    private SendToClaude.Target target = SendToClaude.Target.TERMINAL;

    @Override
    public void setInitializationData(IConfigurationElement config, String propertyName, Object data) {
        target = SendToClaude.Target.of(data);
    }

    @Override
    public Object execute(ExecutionEvent event) {
        try {
            List<IMarker> markers = markersOf(HandlerUtil.getCurrentSelection(event));
            if (markers.isEmpty()) return null;
            String folder = SendToClaude.folderFor(event, target);
            List<String> lines = new ArrayList<>();
            for (IMarker marker : markers) {
                if (!marker.exists()) continue;
                IResource resource = marker.getResource();
                IPath location = resource != null ? resource.getLocation() : null;
                String mention = location != null ? SendToClaudeText.mentionPath(location.toOSString(), folder) : null;
                lines.add(SendToClaudeText.problem(mention, marker.getAttribute(IMarker.LINE_NUMBER, -1),
                        marker.getAttribute(IMarker.SEVERITY, IMarker.SEVERITY_INFO),
                        marker.getAttribute(IMarker.MESSAGE, "")));
            }
            SendToClaude.deliver(event, String.join("\n", lines), target);
        } catch (RuntimeException e) {
            Activator.logError("Failed to send problems to Claude", e);
        }
        return null;
    }

    private static List<IMarker> markersOf(ISelection selection) {
        List<IMarker> out = new ArrayList<>();
        if (selection instanceof IStructuredSelection structured) {
            for (Object element : structured.toList()) {
                IMarker marker = Adapters.adapt(element, IMarker.class);
                if (marker != null) out.add(marker);
            }
        }
        return out;
    }
}
