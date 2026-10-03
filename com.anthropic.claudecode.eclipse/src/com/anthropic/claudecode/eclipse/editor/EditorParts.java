package com.anthropic.claudecode.eclipse.editor;

import java.net.URI;
import java.util.function.Function;

import org.eclipse.core.resources.IResource;
import org.eclipse.core.runtime.IPath;
import org.eclipse.ui.IEditorInput;
import org.eclipse.ui.IEditorPart;
import org.eclipse.ui.IFileEditorInput;
import org.eclipse.ui.IURIEditorInput;
import org.eclipse.ui.IWorkbenchPart;
import org.eclipse.ui.part.MultiPageEditorPart;
import org.eclipse.ui.texteditor.ITextEditor;

/**
 * What a workbench part edits, for the editors that are not a plain text editor on a
 * file on disk (#153). Both lookups answer exactly as a direct check would for those;
 * the fallbacks only run where the direct check finds nothing.
 */
public final class EditorParts {

    private EditorParts() {}

    /**
     * The text editor behind {@code part}, or null when it has none. A multi-page
     * editor (PDE's manifest editor, m2e's POM editor, ABAP's source editors) is not an
     * {@link ITextEditor} itself: the text editor is the page it is showing.
     */
    public static ITextEditor textEditorOf(IWorkbenchPart part) {
        return textEditorOf(part, EditorParts::selectedPageOf);
    }

    /** {@code selectedPage} is a seam: a multi-page editor cannot be built without a workbench. */
    static ITextEditor textEditorOf(IWorkbenchPart part, Function<IEditorPart, Object> selectedPage) {
        if (part instanceof ITextEditor textEditor) return textEditor;
        if (!(part instanceof IEditorPart editor)) return null;
        try {
            if (selectedPage.apply(editor) instanceof ITextEditor page) return page;
            return editor.getAdapter(ITextEditor.class);
        } catch (RuntimeException | LinkageError e) {
            // Another plug-in's editor: one that cannot answer has no text editor to give.
            return null;
        }
    }

    private static Object selectedPageOf(IEditorPart editor) {
        return editor instanceof MultiPageEditorPart multiPage ? multiPage.getSelectedPage() : null;
    }

    /** The path of the file behind an editor input, or null when it is not a file. */
    public static String pathOf(IEditorInput input) {
        if (input instanceof IFileEditorInput fileInput) {
            return pathOf(fileInput.getFile());
        }
        if (input instanceof IURIEditorInput uriInput) {
            return uriInput.getURI().getPath();
        }
        return null;
    }

    /**
     * The disk path of a workspace resource. One kept on a file system other than the
     * local one (ABAP's {@code semanticfs:}, a remote project) has no disk path, so its
     * location URI stands in: nothing can read that from disk, but it names the file.
     */
    public static String pathOf(IResource resource) {
        IPath location = resource.getLocation();
        if (location != null) return location.toOSString();
        URI uri = resource.getLocationURI();
        return uri != null ? uri.toString() : null;
    }
}
