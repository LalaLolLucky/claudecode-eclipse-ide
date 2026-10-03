package com.anthropic.claudecode.eclipse.tools;

import org.eclipse.jface.text.BadLocationException;
import org.eclipse.jface.text.IDocument;
import org.eclipse.jface.text.ITextSelection;
import org.eclipse.jface.viewers.ISelection;
import org.eclipse.ui.IEditorInput;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.texteditor.IDocumentProvider;
import org.eclipse.ui.texteditor.ITextEditor;

import com.anthropic.claudecode.eclipse.editor.EditorParts;
import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.google.gson.JsonObject;

public class GetCurrentSelectionTool implements McpTool {

    @Override
    public String toolName() {
        return "getCurrentSelection";
    }

    @Override
    public String description() {
        return "Get the current text selection in the active Eclipse editor, including file path, line range, and selected text.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");
        schema.add("properties", new JsonObject());
        return schema;
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        return UiHelper.syncCall(() -> {
            try {
                IWorkbenchPage page = UiHelper.getActivePage();
                if (page == null) {
                    return McpToolResult.success(emptySelection());
                }

                ITextEditor textEditor = EditorParts.textEditorOf(page.getActiveEditor());
                if (textEditor == null) {
                    return McpToolResult.success(emptySelection());
                }

                ISelection selection = textEditor.getSelectionProvider().getSelection();
                if (!(selection instanceof ITextSelection textSelection) || textSelection.isEmpty()) {
                    return McpToolResult.success(emptySelection());
                }

                IEditorInput input = textEditor.getEditorInput();
                IDocumentProvider provider = textEditor.getDocumentProvider();
                return McpToolResult.success(describe(EditorUtils.getFilePath(input), textSelection,
                        provider != null ? provider.getDocument(input) : null));
            } catch (Exception e) {
                return McpToolResult.error("Failed to get selection: " + e.getMessage());
            }
        });
    }

    /**
     * The tool's answer for a text selection, in the fields getLatestSelection uses. A bare
     * caret is a valid zero-length selection, so {@link ITextSelection#isEmpty()} is false
     * for it: empty here means no text is highlighted. The caret's file and position are
     * still reported.
     */
    static JsonObject describe(String filePath, ITextSelection selection, IDocument doc) {
        Range range = rangeOf(selection, doc);
        String text = selection.getText();
        JsonObject result = new JsonObject();
        result.addProperty("filePath", filePath != null ? filePath : "");
        result.addProperty("text", text != null ? text : "");
        result.addProperty("startLine", range.startLine());
        result.addProperty("endLine", range.endLine());
        result.addProperty("startColumn", range.startColumn());
        result.addProperty("endColumn", range.endColumn());
        result.addProperty("isEmpty", selection.getLength() <= 0);
        return result;
    }

    /** 1-based lines, 0-based columns in their line. */
    record Range(int startLine, int startColumn, int endLine, int endColumn) {}

    /**
     * Where {@code selection} starts and ends, worked out from the document the way
     * getLatestSelection does, so both tools give the same numbers for one selection. Without
     * a document (or with offsets it no longer has) only the selection's own lines are known.
     */
    static Range rangeOf(ITextSelection selection, IDocument doc) {
        if (doc != null) {
            try {
                int startOffset = selection.getOffset();
                int endOffset = startOffset + Math.max(0, selection.getLength());
                int startLine = doc.getLineOfOffset(startOffset);
                int endLine = doc.getLineOfOffset(endOffset);
                return new Range(startLine + 1, startOffset - doc.getLineOffset(startLine),
                        endLine + 1, endOffset - doc.getLineOffset(endLine));
            } catch (BadLocationException e) {
                // Stale offsets: fall through to the selection's own lines.
            }
        }
        return new Range(selection.getStartLine() + 1, 0, selection.getEndLine() + 1, 0);
    }

    private JsonObject emptySelection() {
        JsonObject result = new JsonObject();
        result.addProperty("text", "");
        result.addProperty("isEmpty", true);
        return result;
    }
}
