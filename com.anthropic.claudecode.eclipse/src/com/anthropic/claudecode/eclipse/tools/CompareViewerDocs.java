package com.anthropic.claudecode.eclipse.tools;

import java.lang.reflect.Field;
import java.lang.reflect.Method;

import org.eclipse.compare.CompareEditorInput;
import org.eclipse.jface.text.IDocument;

/**
 * The text a compare editor's two sides hold right now, read from the viewers' own documents.
 *
 * <p>Eclipse copies a side's text back to its compare item only when the viewer's dirty flag is
 * set, and that flag is reset while the viewer starts up, so by the time a decision is made the
 * item is often stale. The viewer's document is not. Chain:
 * {@code CompareEditorInput.fContentInputPane} (CompareViewerSwitchingPane) → {@code getViewer()}
 * (TextMergeViewer) → {@code fLeft} / {@code fRight} (MergeSourceViewer) →
 * {@code getDocument()} or {@code getSourceViewer().getDocument()}. Every step is reflection on
 * internals, so every failure is a {@code null} for the caller to fall back from.
 */
final class CompareViewerDocs {

    private CompareViewerDocs() {}

    /** The document of one side ({@code "fLeft"} or {@code "fRight"}), or null. */
    static IDocument documentOf(CompareEditorInput input, String sideField) {
        try {
            Field paneField = findDeclaredField(CompareEditorInput.class, "fContentInputPane");
            if (paneField == null) return null;
            paneField.setAccessible(true);
            Object pane = paneField.get(input);
            if (pane == null) return null;

            Method getViewer = findDeclaredMethod(pane.getClass(), "getViewer");
            if (getViewer == null) return null;
            getViewer.setAccessible(true);
            Object viewer = getViewer.invoke(pane);
            if (viewer == null) return null;

            Field sideFieldRef = findDeclaredField(viewer.getClass(), sideField);
            if (sideFieldRef == null) return null;
            sideFieldRef.setAccessible(true);
            Object sideViewer = sideFieldRef.get(viewer);
            if (sideViewer == null) return null;

            Method getDoc = findDeclaredMethod(sideViewer.getClass(), "getDocument");
            if (getDoc != null) {
                getDoc.setAccessible(true);
                Object doc = getDoc.invoke(sideViewer);
                if (doc instanceof IDocument) return (IDocument) doc;
            }

            Method getSV = findDeclaredMethod(sideViewer.getClass(), "getSourceViewer");
            if (getSV != null) {
                getSV.setAccessible(true);
                Object sv = getSV.invoke(sideViewer);
                if (sv != null) {
                    getDoc = findDeclaredMethod(sv.getClass(), "getDocument");
                    if (getDoc != null) {
                        getDoc.setAccessible(true);
                        Object doc = getDoc.invoke(sv);
                        if (doc instanceof IDocument) return (IDocument) doc;
                    }
                }
            }
        } catch (Exception ignored) {}
        return null;
    }

    /** The text of one side, or null. */
    static String textOf(CompareEditorInput input, String sideField) {
        IDocument doc = documentOf(input, sideField);
        return doc == null ? null : doc.get();
    }

    /** Searches {@code cls} and all its superclasses for a declared field. */
    private static Field findDeclaredField(Class<?> cls, String name) {
        for (Class<?> c = cls; c != null; c = c.getSuperclass()) {
            try { return c.getDeclaredField(name); }
            catch (NoSuchFieldException ignored) {}
        }
        return null;
    }

    /** Searches {@code cls} and all its superclasses for a no-arg declared method. */
    private static Method findDeclaredMethod(Class<?> cls, String name) {
        for (Class<?> c = cls; c != null; c = c.getSuperclass()) {
            for (Method m : c.getDeclaredMethods()) {
                if (m.getName().equals(name) && m.getParameterCount() == 0) return m;
            }
        }
        return null;
    }
}
