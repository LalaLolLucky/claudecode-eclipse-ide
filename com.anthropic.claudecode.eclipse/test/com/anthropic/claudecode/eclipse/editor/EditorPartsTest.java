package com.anthropic.claudecode.eclipse.editor;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;

import java.lang.reflect.Proxy;
import java.net.URI;
import java.util.Map;
import java.util.function.Function;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.Path;
import org.eclipse.ui.IEditorInput;
import org.eclipse.ui.IEditorPart;
import org.eclipse.ui.IFileEditorInput;
import org.eclipse.ui.IURIEditorInput;
import org.eclipse.ui.IViewPart;
import org.eclipse.ui.texteditor.ITextEditor;
import org.junit.jupiter.api.Test;

/**
 * Tests for {@link EditorParts}. The parts, inputs and files are {@link Proxy} fakes that answer
 * only the methods named in their map, so the tests run without a workbench or a workspace.
 *
 * <p>Two groups, and the first of each matters most: a plain text editor and a file on the local
 * disk must come back exactly as they did before the fallbacks existed.
 */
class EditorPartsTest {

    /** A fake of {@code type}: each named method returns {@code answers.get(name)} applied to its
     *  arguments, every other method returns null. */
    private static <T> T fake(Class<T> type, Map<String, Function<Object[], Object>> answers) {
        Object proxy = Proxy.newProxyInstance(type.getClassLoader(), new Class<?>[] { type },
                (self, method, args) -> {
                    switch (method.getName()) {
                        case "equals": return self == args[0];
                        case "hashCode": return System.identityHashCode(self);
                        case "toString": return "fake " + type.getSimpleName();
                        default:
                    }
                    Function<Object[], Object> answer = answers.get(method.getName());
                    return answer == null ? null : answer.apply(args);
                });
        return type.cast(proxy);
    }

    private static ITextEditor textEditor() {
        return fake(ITextEditor.class, Map.of());
    }

    // ---- textEditorOf ----

    @Test
    void aTextEditorIsReturnedAsIs() {
        ITextEditor editor = textEditor();
        assertSame(editor, EditorParts.textEditorOf(editor));
    }

    @Test
    void aTextEditorIsNeverAskedForAnAdapter() {
        ITextEditor editor = fake(ITextEditor.class, Map.of("getAdapter", args -> {
            throw new AssertionError("a plain text editor must not be asked to adapt");
        }));
        assertSame(editor, EditorParts.textEditorOf(editor));
    }

    @Test
    void noPartGivesNoEditor() {
        assertNull(EditorParts.textEditorOf(null));
    }

    @Test
    void aViewIsNotAskedForATextEditor() {
        IViewPart view = fake(IViewPart.class, Map.of("getAdapter", args -> {
            throw new AssertionError("only editors are asked to adapt");
        }));
        assertNull(EditorParts.textEditorOf(view));
    }

    @Test
    void anEditorThatAdaptsToATextEditorGivesThatEditor() {
        ITextEditor inner = textEditor();
        IEditorPart outer = fake(IEditorPart.class,
                Map.of("getAdapter", args -> args[0] == ITextEditor.class ? inner : null));
        assertSame(inner, EditorParts.textEditorOf(outer));
    }

    @Test
    void anEditorWithNoTextEditorGivesNone() {
        IEditorPart outer = fake(IEditorPart.class, Map.of());
        assertNull(EditorParts.textEditorOf(outer));
    }

    @Test
    void anEditorWhoseAdapterThrowsGivesNone() {
        IEditorPart outer = fake(IEditorPart.class, Map.of("getAdapter", args -> {
            throw new IllegalStateException("broken third-party editor");
        }));
        assertNull(EditorParts.textEditorOf(outer));
    }

    // A MultiPageEditorPart reads a preference store in its constructor, so it cannot be
    // built here; these pass the page lookup in, as textEditorOf(part) does for a real one.

    @Test
    void aMultiPageEditorGivesItsSelectedTextPage() {
        ITextEditor page = textEditor();
        IEditorPart editor = fake(IEditorPart.class, Map.of("getAdapter", args -> {
            throw new AssertionError("the selected page answers before the adapter is asked");
        }));
        assertSame(page, EditorParts.textEditorOf(editor, e -> page));
    }

    @Test
    void aMultiPageEditorOnANonTextPageFallsBackToItsAdapter() {
        ITextEditor adapted = textEditor();
        IEditorPart editor = fake(IEditorPart.class,
                Map.of("getAdapter", args -> args[0] == ITextEditor.class ? adapted : null));
        assertSame(adapted, EditorParts.textEditorOf(editor, e -> new Object()));
    }

    @Test
    void aMultiPageEditorWithNoTextAnywhereGivesNone() {
        assertNull(EditorParts.textEditorOf(fake(IEditorPart.class, Map.of()), e -> null));
    }

    @Test
    void aPageLookupThatThrowsGivesNone() {
        IEditorPart editor = fake(IEditorPart.class, Map.of());
        assertNull(EditorParts.textEditorOf(editor, e -> {
            throw new IllegalStateException("broken third-party editor");
        }));
    }

    // ---- pathOf ----

    private static IEditorInput fileInput(IPath location, URI locationUri) {
        IFile file = fake(IFile.class,
                Map.of("getLocation", args -> location, "getLocationURI", args -> locationUri));
        return fake(IFileEditorInput.class, Map.of("getFile", args -> file));
    }

    @Test
    void aFileOnDiskGivesItsDiskPath() {
        IPath location = Path.fromOSString(System.getProperty("java.io.tmpdir")).append("A.java");
        IEditorInput input = fileInput(location, location.toFile().toURI());
        assertEquals(location.toOSString(), EditorParts.pathOf(input));
    }

    @Test
    void aFileWithNoDiskLocationGivesItsUri() {
        URI uri = URI.create("semanticfs:/E10_900_user_de/.adt/classlib/classes/zcl_demo/zcl_demo.aclass");
        assertEquals(uri.toString(), EditorParts.pathOf(fileInput(null, uri)));
    }

    @Test
    void aFileWithNoLocationAtAllGivesNoPath() {
        assertNull(EditorParts.pathOf(fileInput(null, null)));
    }

    @Test
    void aUriInputGivesTheUriPath() {
        URI uri = URI.create("file:/C:/ws/notes.txt");
        IEditorInput input = fake(IURIEditorInput.class, Map.of("getURI", args -> uri));
        assertEquals("/C:/ws/notes.txt", EditorParts.pathOf(input));
    }

    @Test
    void anyOtherInputGivesNoPath() {
        assertNull(EditorParts.pathOf(fake(IEditorInput.class, Map.of())));
        assertNull(EditorParts.pathOf((IEditorInput) null));
    }
}
