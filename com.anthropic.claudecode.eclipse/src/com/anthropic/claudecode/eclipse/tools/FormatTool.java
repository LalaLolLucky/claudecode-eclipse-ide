package com.anthropic.claudecode.eclipse.tools;

import java.io.File;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.jface.text.IDocument;
import org.eclipse.jface.text.ITextOperationTarget;
import org.eclipse.jface.text.ITextSelection;
import org.eclipse.jface.text.source.ISourceViewer;
import org.eclipse.jface.viewers.ISelection;
import org.eclipse.ui.IEditorPart;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.ide.IDE;
import org.eclipse.ui.part.FileEditorInput;
import org.eclipse.ui.texteditor.ITextEditor;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Formats a file with its editor's own formatter — the tool form of Source › Format
 * (Ctrl+Shift+F) followed by Save — so the result follows the project's formatter profile,
 * and saving runs whatever save actions the project has configured.
 *
 * <p>It goes through the editor's {@link ITextOperationTarget} with
 * {@link ISourceViewer#FORMAT}, the operation behind that menu item in the Java, C/C++,
 * Python and other text editors alike. Whether a file type has a formatter is asked of the
 * editor ({@code canDoOperation}) at call time rather than guarded at load time, so this
 * depends on nothing beyond {@code org.eclipse.jface.text}, {@code org.eclipse.ui.ide} and
 * {@code org.eclipse.ui.workbench.texteditor}, all hard {@code Require-Bundle}s.
 *
 * <p>An editor holding unsaved changes is refused: formatting and saving would write the
 * user's unsaved edits along with the formatting. An editor the tool opened is closed again;
 * one that was already open keeps its caret.
 */
public class FormatTool implements McpTool {

    @Override
    public String toolName() {
        return "format";
    }

    @Override
    public String description() {
        return "Format a file with Eclipse's formatter for its language (Source > Format), using the "
                + "project's formatter settings, then save it — which also runs the project's save "
                + "actions. The file must be inside a workspace project. Refused when the file has "
                + "unsaved changes in an open editor.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");

        JsonObject props = new JsonObject();
        JsonObject file = new JsonObject();
        file.addProperty("type", "string");
        file.addProperty("description", "Absolute path of the file to format.");
        props.add("file", file);
        schema.add("properties", props);

        JsonArray required = new JsonArray();
        required.add("file");
        schema.add("required", required);
        return schema;
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        try {
            String path = EditorUtils.getString(params, "file", "file");
            if (path == null || path.isBlank()) return McpToolResult.error("Missing required parameter: file");

            IFile file = null;
            for (IFile f : ResourcesPlugin.getWorkspace().getRoot()
                    .findFilesForLocationURI(new File(path).toURI())) {
                if (f.exists()) {
                    file = f;
                    break;
                }
            }
            if (file == null) {
                return McpToolResult.error("Not a file in a workspace project: " + path
                        + ". Formatting goes through the file's editor, so it must be in the workspace.");
            }
            // Claude may just have written the file; make the editor read what is on disk.
            file.refreshLocal(IResource.DEPTH_ZERO, new NullProgressMonitor());

            IFile target = file;
            JsonObject result = UiHelper.syncCall(() -> formatInEditor(target));
            if (result == null) return McpToolResult.error("No workbench window is available.");
            if (result.has("error")) return McpToolResult.error(result.get("error").getAsString());
            result.addProperty("file", path);
            ClaudeCodeView.debug("[format] " + path + " → " + result);
            return McpToolResult.success(result);
        } catch (Exception e) {
            ClaudeCodeView.debug("[format] failed: " + e);
            return McpToolResult.error("format failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }

    /** On the UI thread. Returns {@code {"error": …}} on refusal. */
    private static JsonObject formatInEditor(IFile file) {
        JsonObject out = new JsonObject();
        IWorkbenchPage page = UiHelper.getActivePage();
        if (page == null) {
            out.addProperty("error", "No workbench window is open.");
            return out;
        }
        IEditorPart existing = page.findEditor(new FileEditorInput(file));
        if (existing != null && existing.isDirty()) {
            out.addProperty("error", "The file has unsaved changes in its editor. Save or revert them "
                    + "first — formatting would save them too.");
            return out;
        }

        IEditorPart editor = existing;
        boolean opened = false;
        try {
            if (editor == null) {
                editor = IDE.openEditor(page, file, false);
                opened = true;
            }
            ITextEditor textEditor = editor == null ? null : editor.getAdapter(ITextEditor.class);
            ITextOperationTarget operations = editor == null ? null
                    : editor.getAdapter(ITextOperationTarget.class);
            if (textEditor == null || operations == null || !operations.canDoOperation(ISourceViewer.FORMAT)) {
                out.addProperty("error", "No formatter is available for this file type in "
                        + (editor == null ? "Eclipse" : "the " + editor.getTitle() + " editor") + ".");
                return out;
            }

            IDocument doc = textEditor.getDocumentProvider().getDocument(textEditor.getEditorInput());
            String before = doc.get();
            // FORMAT formats the selection when there is one; an empty selection means the whole file.
            ISelection previous = textEditor.getSelectionProvider().getSelection();
            textEditor.selectAndReveal(0, 0);
            operations.doOperation(ISourceViewer.FORMAT);
            boolean changed = !before.equals(doc.get());
            if (changed) textEditor.doSave(new NullProgressMonitor());

            if (!opened && previous instanceof ITextSelection sel) {
                int offset = Math.min(sel.getOffset(), doc.getLength());
                textEditor.selectAndReveal(offset, 0);
            }
            out.addProperty("changed", changed);
            out.addProperty("saved", changed && !editor.isDirty());
            return out;
        } catch (Exception e) {
            out.addProperty("error", "Could not format: " + e.getClass().getSimpleName() + ": " + e.getMessage());
            return out;
        } finally {
            if (opened && editor != null) page.closeEditor(editor, false);
        }
    }
}
