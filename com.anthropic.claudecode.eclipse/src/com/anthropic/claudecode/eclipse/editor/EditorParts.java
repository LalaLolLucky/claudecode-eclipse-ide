package com.anthropic.claudecode.eclipse.editor;

import java.net.URI;
import java.net.URISyntaxException;
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
     * Whether {@code path} is not a path on the local disk but a URI of another file system
     * ({@code semanticfs:/E10/...} for ABAP, a remote project's): what {@link #pathOf(IResource)}
     * reports for a file that has no disk path. A Windows drive letter ({@code C:\x}) is not a
     * scheme, nor is {@code file:}, which names a disk file.
     */
    public static boolean isNonDiskPath(String path) {
        String scheme = schemeOf(path);
        return scheme != null && !scheme.equalsIgnoreCase("file");
    }

    /** The URI {@code path} names when {@link #isNonDiskPath} says it is one, else null. */
    public static URI nonDiskUriOf(String path) {
        if (!isNonDiskPath(path)) return null;
        try {
            return new URI(path);
        } catch (URISyntaxException e) {
            return null;
        }
    }

    /** What a tool that reads and writes by disk path says when it is handed a URI of another file system. */
    public static String notOnDiskMessage(String path) {
        return path + " is not a file on disk: the \"" + schemeOf(path) + "\" file system keeps it, so this tool, "
            + "which reads and writes files by path, cannot work on it. Use the editor it is open in instead.";
    }

    /** The scheme of a URI-looking path (two or more characters, so not a drive letter), or null. */
    private static String schemeOf(String path) {
        if (path == null) return null;
        int colon = path.indexOf(':');
        if (colon < 2) return null;
        if (!Character.isLetter(path.charAt(0))) return null;
        for (int i = 1; i < colon; i++) {
            char c = path.charAt(i);
            if (!Character.isLetterOrDigit(c) && c != '+' && c != '-' && c != '.') return null;
        }
        return path.substring(0, colon);
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
