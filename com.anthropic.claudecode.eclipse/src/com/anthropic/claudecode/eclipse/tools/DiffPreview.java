package com.anthropic.claudecode.eclipse.tools;

import java.nio.file.Path;

import org.eclipse.compare.CompareConfiguration;
import org.eclipse.compare.CompareEditorInput;
import org.eclipse.compare.CompareUI;
import org.eclipse.compare.structuremergeviewer.DiffNode;
import org.eclipse.compare.structuremergeviewer.Differencer;
import org.eclipse.core.runtime.IProgressMonitor;
import org.eclipse.ui.IEditorPart;
import org.eclipse.ui.IEditorReference;
import org.eclipse.ui.IWorkbenchPage;

import com.anthropic.claudecode.eclipse.editor.UiHelper;

/**
 * Opens an Eclipse diff editor showing current-vs-proposed file content while the in-chat
 * permission decision card is up. Unlike {@link OpenDiffTool} this does NOT block or own the
 * accept/reject decision: the card does. The proposed (right) side can be edited, and what is
 * left there when the card is answered is the change that is approved
 * ({@link #editedContent}); nothing is saved from the editor itself, and closing it asks
 * nothing. It is closed once the user decides.
 */
public final class DiffPreview {

    private DiffPreview() {}

    /** Opens the preview on the UI thread and returns its input (or null on failure). */
    public static CompareEditorInput open(ProposedChange change, String label) {
        final CompareEditorInput[] ref = { null };
        UiHelper.syncCall(() -> {
            try {
                String name = Path.of(change.filePath).getFileName().toString();

                CompareConfiguration cfg = new CompareConfiguration();
                cfg.setLeftEditable(false);
                cfg.setRightEditable(true);
                cfg.setLeftLabel("Current: " + name);
                cfg.setRightLabel("Proposed: " + name);

                CompareItem left  = new CompareItem(name, change.original, false);
                CompareItem right = new CompareItem(name, change.proposed, true);

                CompareEditorInput input = new CompareEditorInput(cfg) {
                    @Override
                    protected Object prepareInput(IProgressMonitor monitor) {
                        return new DiffNode(null, Differencer.CHANGE, null, left, right);
                    }

                    // Nothing here is saved, so the editor is never "unsaved": closing it by
                    // hand must not ask whether to keep the reader's changes to the proposal.
                    @Override
                    public boolean isDirty() { return false; }
                };
                input.setTitle(label);
                CompareUI.openCompareEditor(input);
                ref[0] = input;
            } catch (Exception ignored) {}
            return null;
        });
        return ref[0];
    }

    /**
     * The proposed text as it stands in the preview now, edits included, or null when it cannot
     * be read (the viewer is not up, or Eclipse's internals differ from what is expected).
     *
     * <p>Which of the viewer's two fields holds the editable side varies with Eclipse's layout
     * (the same ambiguity {@link OpenDiffTool} works around), so both are read: the side that is
     * neither the file's text nor the proposal is the one that was edited. Both back at the
     * file's text is a proposal put back as it was; neither changed is the proposal itself.
     */
    public static String editedContent(CompareEditorInput input, ProposedChange change) {
        if (input == null) return null;
        return UiHelper.syncCall(() -> {
            String a = CompareViewerDocs.textOf(input, "fLeft");
            String b = CompareViewerDocs.textOf(input, "fRight");
            if (a == null || b == null) return null;
            if (a.equals(change.original) && b.equals(change.original)) return change.original;
            for (String t : new String[] { a, b }) {
                if (!t.equals(change.original) && !t.equals(change.proposed)) return t;
            }
            return change.proposed;
        });
    }

    /** Closes a preview opened by {@link #open}. Safe to call with null. */
    public static void close(CompareEditorInput input) {
        if (input == null) return;
        UiHelper.syncCall(() -> {
            IWorkbenchPage page = UiHelper.getActivePage();
            if (page != null) {
                for (IEditorReference ref : page.getEditorReferences()) {
                    IEditorPart editor = ref.getEditor(false);
                    if (editor != null && input.equals(editor.getEditorInput())) {
                        page.closeEditor(editor, false);
                        break;
                    }
                }
            }
            return null;
        });
    }
}
