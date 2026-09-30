package com.anthropic.claudecode.eclipse.tools.jdt;

import java.io.ByteArrayInputStream;
import java.io.File;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.core.runtime.Platform;
import org.eclipse.core.runtime.preferences.DefaultScope;
import org.eclipse.jdt.core.IAccessRule;
import org.eclipse.jdt.core.ICompilationUnit;
import org.eclipse.jdt.core.JavaCore;
import org.eclipse.jdt.core.manipulation.CodeStyleConfiguration;
import org.eclipse.jdt.core.manipulation.JavaManipulation;
import org.eclipse.jdt.core.manipulation.OrganizeImportsOperation;
import org.eclipse.jdt.core.search.TypeNameMatch;
import org.eclipse.jface.text.Document;
import org.eclipse.text.edits.TextEdit;
import org.eclipse.ui.IEditorPart;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.part.FileEditorInput;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Organize Imports (Ctrl+Shift+O) on a Java file: adds the imports its code needs, removes
 * unused ones, and orders them by the project's import settings. Adapted from JDT Bridge's
 * {@code RefactoringHandler.handleOrganizeImports}: the edit is computed on the file's
 * source and written back to the file, rather than through a working copy.
 *
 * <p>Where a simple name matches several types, the dialog would ask. There is no one to
 * ask here, so the likeliest candidate is taken ({@link #rank}) — and every such choice is
 * reported with its alternatives, so a wrong pick can be corrected.
 *
 * <p>Needs {@code org.eclipse.jdt.core} and {@code org.eclipse.jdt.core.manipulation};
 * registered only when both are installed.
 */
public class OrganizeImportsTool implements McpTool {

	@Override
	public String toolName() {
		return "organizeImports";
	}

	@Override
	public String description() {
		return "Organize the imports of a Java file (Source > Organize Imports): add missing imports, "
				+ "remove unused ones, and order them by the project's settings, then save. When a "
				+ "name matches several types the likeliest is chosen and reported under 'ambiguous' "
				+ "with the alternatives. Refused when the file has unsaved changes in an editor.";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject schema = new JsonObject();
		schema.addProperty("type", "object");
		JsonObject props = new JsonObject();
		JsonObject file = new JsonObject();
		file.addProperty("type", "string");
		file.addProperty("description", "Absolute path of the .java file.");
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
			String path = params.has("file") && !params.get("file").isJsonNull() ? params.get("file").getAsString() : null;
			if (path == null || path.isBlank()) return McpToolResult.error("Missing required parameter: file");

			IFile file = null;
			for (IFile f : ResourcesPlugin.getWorkspace().getRoot().findFilesForLocationURI(new File(path).toURI())) {
				if (f.exists()) {
					file = f;
					break;
				}
			}
			if (file == null) return McpToolResult.error("Not a file in a workspace project: " + path);
			file.refreshLocal(IResource.DEPTH_ZERO, new NullProgressMonitor());

			ICompilationUnit cu = JavaCore.createCompilationUnitFrom(file);
			if (cu == null || !cu.exists()) {
				return McpToolResult.error("Not a Java source file on a project's build path: " + path);
			}

			IFile target = file;
			Boolean dirty = UiHelper.syncCall(() -> {
				IWorkbenchPage page = UiHelper.getActivePage();
				IEditorPart editor = page == null ? null : page.findEditor(new FileEditorInput(target));
				return editor != null && editor.isDirty();
			});
			if (Boolean.TRUE.equals(dirty)) {
				return McpToolResult.error("The file has unsaved changes in its editor. Save or revert them first.");
			}

			JsonArray ambiguous = new JsonArray();
			OrganizeImportsOperation.IChooseImportQuery query = (openChoices, ranges) -> {
				TypeNameMatch[] chosen = new TypeNameMatch[openChoices.length];
				for (int i = 0; i < openChoices.length; i++) {
					TypeNameMatch[] candidates = openChoices[i].clone();
					// Stable sort: JDT's own order breaks ties.
					java.util.Arrays.sort(candidates, java.util.Comparator.comparingInt(OrganizeImportsTool::rank));
					chosen[i] = candidates[0];
					JsonObject a = new JsonObject();
					a.addProperty("chosen", candidates[0].getFullyQualifiedName());
					JsonArray alternatives = new JsonArray();
					for (int k = 1; k < candidates.length; k++) alternatives.add(candidates[k].getFullyQualifiedName());
					a.add("alternatives", alternatives);
					ambiguous.add(a);
				}
				return chosen;
			};

			ensurePreferencesInitialized();
			String source = cu.getSource();
			OrganizeImportsOperation op = new OrganizeImportsOperation(cu, null, true, false, true, query);
			TextEdit edit = op.createTextEdit(new NullProgressMonitor());
			int added = op.getNumberOfImportsAdded();
			int removed = op.getNumberOfImportsRemoved();

			boolean changed = false;
			if (edit != null) {
				Document doc = new Document(source);
				edit.apply(doc);
				if (!doc.get().equals(source)) {
					file.setContents(new ByteArrayInputStream(doc.get().getBytes(file.getCharset())),
							IResource.FORCE | IResource.KEEP_HISTORY, new NullProgressMonitor());
					changed = true;
				}
			}

			JsonObject out = new JsonObject();
			out.addProperty("file", path);
			out.addProperty("added", added);
			out.addProperty("removed", removed);
			out.addProperty("changed", changed);
			if (!ambiguous.isEmpty()) out.add("ambiguous", ambiguous);
			if (op.getParseError() != null) {
				out.addProperty("note", "The file has a syntax error, so some imports may be missing: "
						+ op.getParseError().getMessage());
			}
			ClaudeCodeView.debug("[organizeImports] " + path + " +" + added + " -" + removed);
			return McpToolResult.success(out);
		} catch (Exception | LinkageError e) {
			ClaudeCodeView.debug("[organizeImports] failed: " + e);
			return McpToolResult.error("organizeImports failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
		}
	}

	/**
	 * Lower is likelier to be meant. JDT offers every visible match, including types the
	 * project may not use (access rules) and JDK internals; with no one to ask, prefer an
	 * accessible, public-API type, and the {@code java.*} packages among those.
	 */
	private static int rank(TypeNameMatch match) {
		int rank = 0;
		int access = match.getAccessibility();
		if (access == IAccessRule.K_NON_ACCESSIBLE) rank += 100;
		else if (access == IAccessRule.K_DISCOURAGED) rank += 50;
		String pkg = match.getPackageName();
		if (pkg.startsWith("com.sun.") || pkg.startsWith("sun.") || pkg.startsWith("jdk.internal")
				|| pkg.contains(".internal.") || pkg.endsWith(".internal")) {
			rank += 20;
		}
		if (pkg.startsWith("java.")) rank -= 2;
		else if (pkg.startsWith("javax.")) rank -= 1;
		return rank;
	}

	/**
	 * From JDT Bridge: the import-order preferences live under a node JDT UI names when it
	 * starts. With JDT UI installed, leave it to set that; without it (JDT core and
	 * manipulation only), name the node ourselves and give it JDT's defaults, or reading
	 * the preferences hangs.
	 */
	private static void ensurePreferencesInitialized() {
		if (JavaManipulation.getPreferenceNodeId() != null) return;
		org.osgi.framework.Bundle jdtUi = Platform.getBundle("org.eclipse.jdt.ui");
		if (jdtUi != null && jdtUi.getState() != org.osgi.framework.Bundle.UNINSTALLED) return;
		String nodeId = "org.eclipse.jdt.core.manipulation";
		JavaManipulation.setPreferenceNodeId(nodeId);
		var defaults = DefaultScope.INSTANCE.getNode(nodeId);
		defaults.put(CodeStyleConfiguration.ORGIMPORTS_IMPORTORDER, "java;javax;org;com");
		defaults.put(CodeStyleConfiguration.ORGIMPORTS_ONDEMANDTHRESHOLD, "99");
		defaults.put(CodeStyleConfiguration.ORGIMPORTS_STATIC_ONDEMANDTHRESHOLD, "99");
	}
}
