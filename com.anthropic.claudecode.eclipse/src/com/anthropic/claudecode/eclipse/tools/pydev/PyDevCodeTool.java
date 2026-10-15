package com.anthropic.claudecode.eclipse.tools.pydev;

import java.io.File;
import java.io.InputStream;
import java.nio.charset.Charset;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Map;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IWorkspaceRoot;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.core.runtime.Path;
import org.eclipse.jface.text.Document;
import org.python.pydev.ast.item_pointer.ItemPointer;
import org.python.pydev.ast.refactoring.AbstractPyRefactoring;
import org.python.pydev.ast.refactoring.IPyRefactoring;
import org.python.pydev.ast.refactoring.IPyRefactoring2;
import org.python.pydev.ast.refactoring.RefactoringRequest;
import org.python.pydev.core.IPythonNature;
import org.python.pydev.core.docutils.PySelection;
import org.python.pydev.parser.visitors.scope.ASTEntry;
import org.python.pydev.plugin.nature.PythonNature;
import org.python.pydev.shared_core.structure.Tuple;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.tools.SourcePosition;
import com.anthropic.claudecode.eclipse.tools.ToolSupport;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Python through PyDev's own Go to Definition and Find References: where the name at a position is
 * defined, and every place it is used. PyDev has no public API in the OSGi sense; this uses the
 * classes those two commands use (IPyRefactoring#findDefinition, IPyRefactoring2#findAllOccurrences
 * with a RefactoringRequest), whose signatures are the same in PyDev 9.3 (2022) and 13.1 (2025).
 * PyDev is optional; the registry creates this tool only when its bundles are there.
 */
public class PyDevCodeTool implements McpTool {

	private static final List<String> ACTIONS = List.of("definition", "references");

	/** The most places listed. */
	private static final int MAX_PLACES = 500;

	@Override
	public String toolName() {
		return "pydevCode";
	}

	@Override
	public String description() {
		return "Python through PyDev, as Go to Definition (F3) and Find References do. Name the place by 'file' "
				+ "(workspace path such as /proj/src/mod.py, or a full path) with 'line' and 'column' (both from 1) "
				+ "on a name. action='definition' gives where that name is defined; action='references' lists "
				+ "every place it is used, in the project and the ones it is related to. The file must be in a "
				+ "PyDev project with an interpreter configured.";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject props = new JsonObject();
		props.add("action", ToolSupport.choice("definition or references.", ACTIONS));
		props.add("file", ToolSupport.prop("string", "The Python file: workspace path such as /proj/src/mod.py, or a full path."));
		props.add("line", ToolSupport.prop("integer", "The line of the name, from 1."));
		props.add("column", ToolSupport.prop("integer", "The column of the name, from 1."));
		return ToolSupport.schema(props, "action", "file", "line", "column");
	}

	@Override
	public McpToolResult execute(JsonObject params) {
		try {
			String action = ToolSupport.str(params, "action");
			if (action == null || !ACTIONS.contains(action)) {
				return McpToolResult.error("'action' must be one of: " + String.join(", ", ACTIONS) + ".");
			}
			String named = ToolSupport.str(params, "file");
			if (named == null) return McpToolResult.error("'file' is required: a workspace path such as /proj/src/mod.py, or a full path.");
			Integer line = intOf(params, "line"), column = intOf(params, "column");
			if (line == null || column == null) return McpToolResult.error("'line' and 'column' (both from 1) are required.");
			ClaudeCodeView.debug("[pydevCode] " + params);
			IFile file = fileOf(named);
			if (file == null) return McpToolResult.error("No workspace file at '" + named + "'.");
			IPythonNature nature = PythonNature.getPythonNature(file.getProject());
			if (nature == null) return McpToolResult.error(file.getProject().getName() + " is not a PyDev project.");
			String text = text(file);
			int offset = SourcePosition.offset(text, line, column);
			if (offset < 0) return McpToolResult.error("The file has no line " + line + " column " + column + ".");
			if (SourcePosition.identifierAt(text, offset) == null) {
				return McpToolResult.error("There is no name at line " + line + " column " + column + ".");
			}

			// PySelection counts lines and columns from 0.
			PySelection selection = new PySelection(new Document(text), line - 1, column - 1);
			File onDisk = file.getLocation().toFile();
			RefactoringRequest request = new RefactoringRequest(onDisk, selection, new NullProgressMonitor(), nature, null);
			request.fillActivationTokenAndQualifier();
			IPyRefactoring refactoring = AbstractPyRefactoring.getPyRefactoring();
			JsonObject out = new JsonObject();
			out.addProperty("name", request.qualifier);
			if ("definition".equals(action)) {
				out.add("definitions", definitions(refactoring.findDefinition(request)));
			} else {
				if (!(refactoring instanceof IPyRefactoring2 finder)) {
					return McpToolResult.error("This PyDev cannot find references.");
				}
				out.add("references", references(finder.findAllOccurrences(request)));
			}
			return McpToolResult.success(out);
		} catch (Exception | LinkageError e) {
			ClaudeCodeView.debug("[pydevCode] failed: " + e);
			return McpToolResult.error("pydevCode failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
		}
	}

	private static Integer intOf(JsonObject params, String key) {
		return params.has(key) && !params.get(key).isJsonNull() ? params.get(key).getAsInt() : null;
	}

	private static IFile fileOf(String named) {
		IWorkspaceRoot root = ResourcesPlugin.getWorkspace().getRoot();
		try {
			java.nio.file.Path full = java.nio.file.Paths.get(named);
			if (full.isAbsolute() && java.nio.file.Files.exists(full)) {
				IFile[] found = root.findFilesForLocationURI(full.toUri());
				if (found.length > 0) return found[0];
			}
		} catch (RuntimeException notAFullPath) {
			// read as a workspace path below
		}
		try {
			IFile file = root.getFile(new Path(named));
			return file.exists() ? file : null;
		} catch (IllegalArgumentException notInTheWorkspace) {
			return null;
		}
	}

	private static String text(IFile file) throws Exception {
		try (InputStream in = file.getContents()) {
			return new String(in.readAllBytes(), Charset.forName(file.getCharset()));
		}
	}

	/** Where each definition is; PyDev's locations count from 0. */
	private static JsonArray definitions(ItemPointer[] pointers) {
		JsonArray out = new JsonArray();
		if (pointers == null) return out;
		for (ItemPointer p : pointers) {
			if (out.size() >= MAX_PLACES) break;
			JsonObject j = new JsonObject();
			j.addProperty("file", pathOf(p.file));
			if (p.start != null && p.start.line >= 0) {
				j.addProperty("line", p.start.line + 1);
				j.addProperty("column", p.start.column + 1);
			}
			if (p.zipFilePath != null) j.addProperty("inArchive", p.zipFilePath);
			out.add(j);
		}
		return out;
	}

	/** Every occurrence, by file; the parser's nodes count lines and columns from 1. */
	private static JsonArray references(Map<Tuple<String, File>, HashSet<ASTEntry>> found) {
		JsonArray out = new JsonArray();
		if (found == null) return out;
		for (Map.Entry<Tuple<String, File>, HashSet<ASTEntry>> e : found.entrySet()) {
			List<ASTEntry> entries = new ArrayList<>(e.getValue());
			entries.sort((a, b) -> a.node.beginLine != b.node.beginLine ? a.node.beginLine - b.node.beginLine
					: a.node.beginColumn - b.node.beginColumn);
			for (ASTEntry entry : entries) {
				if (out.size() >= MAX_PLACES) return out;
				JsonObject j = new JsonObject();
				j.addProperty("file", e.getKey().o2 != null ? e.getKey().o2.getPath() : e.getKey().o1);
				if (e.getKey().o1 != null) j.addProperty("module", e.getKey().o1);
				j.addProperty("line", entry.node.beginLine);
				j.addProperty("column", entry.node.beginColumn);
				out.add(j);
			}
		}
		return out;
	}

	private static String pathOf(Object file) {
		if (file instanceof File f) return f.getPath();
		if (file instanceof IFile f) return f.getLocation() != null ? f.getLocation().toOSString() : f.getFullPath().toString();
		return String.valueOf(file);
	}
}
