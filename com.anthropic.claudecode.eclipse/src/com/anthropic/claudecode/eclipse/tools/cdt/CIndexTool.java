package com.anthropic.claudecode.eclipse.tools.cdt;

import java.io.InputStream;
import java.nio.charset.Charset;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import org.eclipse.cdt.core.CCorePlugin;
import org.eclipse.cdt.core.dom.ast.IASTTranslationUnit;
import org.eclipse.cdt.core.dom.ast.IFunction;
import org.eclipse.cdt.core.dom.rewrite.MacroExpansionExplorer;
import org.eclipse.cdt.core.index.IIndex;
import org.eclipse.cdt.core.index.IIndexBinding;
import org.eclipse.cdt.core.index.IIndexFile;
import org.eclipse.cdt.core.index.IIndexFileLocation;
import org.eclipse.cdt.core.index.IIndexInclude;
import org.eclipse.cdt.core.index.IIndexManager;
import org.eclipse.cdt.core.index.IIndexName;
import org.eclipse.cdt.core.index.IndexFilter;
import org.eclipse.cdt.core.index.IndexLocationFactory;
import org.eclipse.cdt.core.model.CoreModel;
import org.eclipse.cdt.core.model.ICElement;
import org.eclipse.cdt.core.model.ICProject;
import org.eclipse.cdt.core.model.ITranslationUnit;
import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IWorkspaceRoot;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.CoreException;
import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.core.runtime.Path;
import org.eclipse.jface.text.Region;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.tools.SourcePosition;
import com.anthropic.claudecode.eclipse.tools.ToolSupport;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * C and C++ through CDT's index (the one behind Open Declaration, References, the Call and Include
 * Browsers): where a symbol is defined, declared and used, which functions call it, what a file
 * includes and what includes it, and how a macro expands. Public CDT API only (cdt.core's index,
 * model and MacroExpansionExplorer, the same in CDT 7.4 and 9.3). CDT is optional; the registry
 * creates this tool only when org.eclipse.cdt.core is there.
 */
public class CIndexTool implements McpTool {

	private static final List<String> ACTIONS = List.of("references", "callers", "includes", "macroExpansion");

	/** The most names listed for one symbol, and include levels followed. */
	private static final int MAX_NAMES = 500;
	private static final int MAX_DEPTH = 10;

	/** How long the indexer is waited for before the index is read as it stands. */
	private static final int INDEX_WAIT_MS = 60_000;

	@Override
	public String toolName() {
		return "cIndex";
	}

	@Override
	public String description() {
		return "C and C++ through CDT's index, as Open Declaration, References and the Call and Include Browsers "
				+ "see it. Name the symbol by 'file' with 'line' and 'column' (both from 1) on its name, or by "
				+ "'symbol', its qualified name such as ns::Klass::method. action='references' lists where it is "
				+ "defined, declared and used; action='callers' lists the functions whose bodies call it. "
				+ "action='includes' lists what 'file' includes and what includes it, 'depth' levels deep "
				+ "(1 when left out). action='macroExpansion' expands the macro at 'file' 'line' 'column', step "
				+ "by step. The index has to be built (CDT does it in the background); local variables are not "
				+ "in it.";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject props = new JsonObject();
		props.add("action", ToolSupport.choice("references, callers, includes or macroExpansion.", ACTIONS));
		props.add("file", ToolSupport.prop("string", "A C/C++ file: workspace path such as /proj/src/a.c, or a full path."));
		props.add("line", ToolSupport.prop("integer", "With 'file': the line of the name, from 1."));
		props.add("column", ToolSupport.prop("integer", "With 'file': the column of the name, from 1."));
		props.add("symbol", ToolSupport.prop("string", "references/callers: instead of a position, the qualified name, e.g. ns::f."));
		props.add("depth", ToolSupport.prop("integer", "includes: how many levels to follow, 1 to " + MAX_DEPTH + "."));
		return ToolSupport.schema(props, "action");
	}

	@Override
	public McpToolResult execute(JsonObject params) {
		ClaudeCodeView.debug("[cIndex] " + params);
		try {
			String action = ToolSupport.str(params, "action");
			if (action == null || !ACTIONS.contains(action)) {
				return McpToolResult.error("'action' must be one of: " + String.join(", ", ACTIONS) + ".");
			}
			return switch (action) {
				case "includes" -> includes(params);
				case "macroExpansion" -> macroExpansion(params);
				default -> names(params, "callers".equals(action));
			};
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
			return McpToolResult.error("cIndex was interrupted.");
		} catch (Refusal r) {
			return McpToolResult.error(r.getMessage());
		} catch (Exception | LinkageError e) {
			ClaudeCodeView.debug("[cIndex] failed: " + e);
			return McpToolResult.error("cIndex failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
		}
	}

	private static final class Refusal extends RuntimeException {
		private static final long serialVersionUID = 1L;

		Refusal(String message) {
			super(message);
		}
	}

	// ── the file, the project and its index ─────────────────────────────────

	private static IFile fileOf(JsonObject params) {
		String named = ToolSupport.str(params, "file");
		if (named == null) throw new Refusal("'file' is required: a workspace path such as /proj/src/a.c, or a full path.");
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
			if (file.exists()) return file;
		} catch (IllegalArgumentException notInTheWorkspace) {
			// refused below
		}
		throw new Refusal("No workspace file at '" + named + "'.");
	}

	private static ICProject projectOf(IFile file) {
		ICProject project = CoreModel.getDefault().create(file.getProject());
		if (project == null || !project.exists()) {
			throw new Refusal(file.getProject().getName() + " is not a C/C++ project, so CDT has no index for it.");
		}
		return project;
	}

	/**
	 * The project's index, with {@code file} in it when CDT can index it: what the indexer is
	 * doing is waited for, a while, and a file the index does not hold has its project indexed
	 * again (Index &gt; Rebuild), which a project imported or set up a moment ago needs (seen live:
	 * an index made before the project had its .cproject stayed empty).
	 */
	private static IIndex indexFor(ICProject project, IFile file) throws CoreException, InterruptedException {
		IIndexManager manager = CCorePlugin.getIndexManager();
		if (!manager.isProjectIndexed(project)) manager.reindex(project);
		manager.joinIndexer(INDEX_WAIT_MS, new NullProgressMonitor());
		IIndex index = manager.getIndex(project, IIndexManager.ADD_DEPENDENCIES | IIndexManager.ADD_DEPENDENT);
		boolean held;
		index.acquireReadLock();
		try {
			held = index.getFiles(IndexLocationFactory.getWorkspaceIFL(file)).length > 0;
		} finally {
			index.releaseReadLock();
		}
		if (held) return index;
		manager.reindex(project);
		manager.joinIndexer(INDEX_WAIT_MS, new NullProgressMonitor());
		return manager.getIndex(project, IIndexManager.ADD_DEPENDENCIES | IIndexManager.ADD_DEPENDENT);
	}

	/** Every C/C++ project's index; one whose index holds no file at all is indexed again first. */
	private static IIndex workspaceIndex() throws CoreException, InterruptedException {
		IIndexManager manager = CCorePlugin.getIndexManager();
		ICProject[] projects = CoreModel.getDefault().getCModel().getCProjects();
		manager.joinIndexer(INDEX_WAIT_MS, new NullProgressMonitor());
		boolean again = false;
		for (ICProject p : projects) {
			IIndex own = manager.getIndex(p);
			own.acquireReadLock();
			try {
				if (!manager.isProjectIndexed(p) || own.getAllFiles().length == 0) {
					manager.reindex(p);
					again = true;
				}
			} finally {
				own.releaseReadLock();
			}
		}
		if (again) manager.joinIndexer(INDEX_WAIT_MS, new NullProgressMonitor());
		return manager.getIndex(projects);
	}

	private static String text(IFile file) throws Exception {
		try (InputStream in = file.getContents()) {
			return new String(in.readAllBytes(), Charset.forName(file.getCharset()));
		}
	}

	/** The offset of 'line' 'column' in the file, and the identifier there. */
	private static int[] identifierOf(JsonObject params, String text) {
		Integer line = intOf(params, "line"), column = intOf(params, "column");
		if (line == null || column == null) throw new Refusal("'line' and 'column' (both from 1) are required with 'file'.");
		int offset = SourcePosition.offset(text, line, column);
		if (offset < 0) throw new Refusal("The file has no line " + line + " column " + column + ".");
		int[] word = SourcePosition.identifierAt(text, offset);
		if (word == null) throw new Refusal("There is no name at line " + line + " column " + column + ".");
		return word;
	}

	private static Integer intOf(JsonObject params, String key) {
		return params.has(key) && !params.get(key).isJsonNull() ? params.get(key).getAsInt() : null;
	}

	// ── references / callers ───────────────────────────────────────────────

	private McpToolResult names(JsonObject params, boolean callers) throws Exception {
		String symbol = ToolSupport.str(params, "symbol");
		IIndex index;
		IFile file = null;
		String text = null;
		if (symbol != null) {
			index = workspaceIndex();
		} else {
			file = fileOf(params);
			index = indexFor(projectOf(file), file);
			text = text(file);
		}
		index.acquireReadLock();
		try {
			List<IIndexBinding> bindings = new ArrayList<>();
			if (symbol != null) {
				char[][] qualified = splitQualified(symbol);
				for (IIndexBinding b : index.findBindings(qualified, IndexFilter.ALL_DECLARED, new NullProgressMonitor())) bindings.add(b);
				if (bindings.isEmpty()) throw new Refusal("No symbol named '" + symbol + "' in the index.");
			} else {
				int[] word = identifierOf(params, text);
				IIndexBinding binding = bindingAt(index, file, word);
				if (binding == null) {
					throw new Refusal("'" + text.substring(word[0], word[1]) + "' is not in the index: it may be a local "
							+ "name, or the file is not indexed yet.");
				}
				bindings.add(binding);
			}

			JsonObject out = new JsonObject();
			JsonArray symbols = new JsonArray();
			Texts texts = new Texts();
			for (IIndexBinding binding : bindings) {
				JsonObject s = new JsonObject();
				s.addProperty("symbol", String.join("::", binding.getQualifiedName()));
				s.addProperty("kind", kindOf(binding));
				if (callers) {
					s.add("callers", callersOf(index, binding, texts));
				} else {
					s.add("definitions", places(index.findDefinitions(binding), texts));
					s.add("declarations", places(index.findDeclarations(binding), texts));
					s.add("references", places(index.findReferences(binding), texts));
				}
				symbols.add(s);
			}
			out.add("symbols", symbols);
			return McpToolResult.success(out);
		} finally {
			index.releaseReadLock();
		}
	}

	private static char[][] splitQualified(String symbol) {
		String[] parts = symbol.trim().replaceFirst("^::", "").split("::");
		char[][] out = new char[parts.length][];
		for (int i = 0; i < parts.length; i++) out[i] = parts[i].trim().toCharArray();
		return out;
	}

	/** The binding of the indexed name that covers the identifier at {@code word}. */
	private static IIndexBinding bindingAt(IIndex index, IFile file, int[] word) throws CoreException {
		IIndexFileLocation location = IndexLocationFactory.getWorkspaceIFL(file);
		for (IIndexFile indexed : index.getFiles(location)) {
			for (IIndexName name : indexed.findNames(word[0], word[1] - word[0])) {
				if (name.getNodeOffset() <= word[0] && name.getNodeOffset() + name.getNodeLength() >= word[1]) {
					IIndexBinding binding = index.findBinding(name);
					if (binding != null) return binding;
				}
			}
		}
		return null;
	}

	private static String kindOf(IIndexBinding binding) {
		String simple = binding.getClass().getSimpleName().replaceFirst("^PDOM", "").replaceFirst("^CPP", "").replaceFirst("^C", "");
		if (binding instanceof IFunction) return "function";
		return simple.isEmpty() ? "symbol" : Character.toLowerCase(simple.charAt(0)) + simple.substring(1);
	}

	private static JsonArray places(IIndexName[] names, Texts texts) throws CoreException {
		JsonArray out = new JsonArray();
		for (IIndexName name : names) {
			if (out.size() >= MAX_NAMES) break;
			out.add(place(name, texts));
		}
		return out;
	}

	/** A name's file, line and column. The index keeps a name's offset but not its line (its
	 *  file location says line 0, seen live), so the line is counted in the file's text. */
	private static JsonObject place(IIndexName name, Texts texts) throws CoreException {
		IIndexFileLocation file = name.getFile().getLocation();
		JsonObject j = new JsonObject();
		j.addProperty("file", pathOf(file));
		String text = texts.of(file);
		if (text != null && name.getNodeOffset() >= 0 && name.getNodeOffset() <= text.length()) {
			int[] at = SourcePosition.lineAndColumn(text, name.getNodeOffset());
			j.addProperty("line", at[0]);
			j.addProperty("column", at[1]);
		}
		return j;
	}

	/** The files' texts a call has read, each read once. */
	private static final class Texts {
		private final Map<String, String> read = new LinkedHashMap<>();

		String of(IIndexFileLocation location) {
			return read.computeIfAbsent(location.getURI().toString(), k -> {
				try {
					java.nio.file.Path path = java.nio.file.Paths.get(location.getURI());
					try {
						return java.nio.file.Files.readString(path);
					} catch (java.nio.charset.CharacterCodingException notUtf8) {
						return java.nio.file.Files.readString(path, java.nio.charset.StandardCharsets.ISO_8859_1);
					}
				} catch (Exception | Error unreadable) {
					return null;
				}
			});
		}
	}

	private static String pathOf(IIndexFileLocation location) {
		if (location.getFullPath() != null) return location.getFullPath();
		IPath absolute = IndexLocationFactory.getAbsolutePath(location);
		return absolute != null ? absolute.toOSString() : location.getURI().toString();
	}

	/** The functions whose definitions hold a reference to {@code binding}, each with the lines. */
	private static JsonArray callersOf(IIndex index, IIndexBinding binding, Texts texts) throws CoreException {
		Map<String, JsonObject> byCaller = new LinkedHashMap<>();
		for (IIndexName reference : index.findReferences(binding)) {
			IIndexName enclosing = reference.getEnclosingDefinition();
			IIndexBinding caller = enclosing != null ? index.findBinding(enclosing) : null;
			String key = caller != null ? String.join("::", caller.getQualifiedName()) : "(outside any function)";
			JsonObject entry = byCaller.computeIfAbsent(key, k -> {
				JsonObject j = new JsonObject();
				j.addProperty("caller", k);
				try {
					if (enclosing != null) j.add("definedAt", place(enclosing, texts));
				} catch (CoreException e) {
					// left out
				}
				j.add("calls", new JsonArray());
				return j;
			});
			JsonArray calls = entry.getAsJsonArray("calls");
			if (calls.size() < MAX_NAMES) calls.add(place(reference, texts));
			if (byCaller.size() >= MAX_NAMES) break;
		}
		JsonArray out = new JsonArray();
		byCaller.values().forEach(out::add);
		return out;
	}

	// ── includes ────────────────────────────────────────────────────────────

	private McpToolResult includes(JsonObject params) throws Exception {
		IFile file = fileOf(params);
		Integer depth = intOf(params, "depth");
		int levels = depth == null ? 1 : depth;
		if (levels < 1 || levels > MAX_DEPTH) throw new Refusal("'depth' must be from 1 to " + MAX_DEPTH + ".");
		IIndex index = indexFor(projectOf(file), file);
		index.acquireReadLock();
		try {
			IIndexFile[] indexed = index.getFiles(IndexLocationFactory.getWorkspaceIFL(file));
			if (indexed.length == 0) throw new Refusal(file.getFullPath() + " is not in the index yet.");
			JsonObject out = new JsonObject();
			out.addProperty("file", file.getFullPath().toString());
			JsonArray includes = new JsonArray();
			for (IIndexInclude include : index.findIncludes(indexed[0], levels)) {
				if (includes.size() >= MAX_NAMES) break;
				JsonObject j = new JsonObject();
				j.addProperty("name", include.getFullName());
				j.addProperty("from", pathOf(include.getIncludedByLocation()));
				if (include.isResolved() && include.getIncludesLocation() != null) j.addProperty("resolvedTo", pathOf(include.getIncludesLocation()));
				else j.addProperty("unresolved", true);
				if (include.isSystemInclude()) j.addProperty("system", true);
				if (!include.isActive()) j.addProperty("inactive", true);
				includes.add(j);
			}
			out.add("includes", includes);
			JsonArray includedBy = new JsonArray();
			for (IIndexInclude include : index.findIncludedBy(indexed[0], levels)) {
				if (includedBy.size() >= MAX_NAMES) break;
				JsonObject j = new JsonObject();
				j.addProperty("file", pathOf(include.getIncludedByLocation()));
				j.addProperty("includes", include.getFullName());
				includedBy.add(j);
			}
			out.add("includedBy", includedBy);
			return McpToolResult.success(out);
		} finally {
			index.releaseReadLock();
		}
	}

	// ── macroExpansion ──────────────────────────────────────────────────────

	private McpToolResult macroExpansion(JsonObject params) throws Exception {
		IFile file = fileOf(params);
		ICProject project = projectOf(file);
		ICElement element = CoreModel.getDefault().create(file);
		if (!(element instanceof ITranslationUnit unit)) throw new Refusal(file.getFullPath() + " is not a C/C++ source or header file.");
		String text = text(file);
		int[] word = identifierOf(params, text);
		IIndex index = indexFor(project, file);
		index.acquireReadLock();
		try {
			IASTTranslationUnit ast = unit.getAST(index, ITranslationUnit.AST_SKIP_INDEXED_HEADERS);
			if (ast == null) throw new Refusal("CDT could not parse " + file.getFullPath() + ".");
			MacroExpansionExplorer explorer = MacroExpansionExplorer.create(ast, new Region(word[0], word[1] - word[0]));
			int steps = explorer.getExpansionStepCount();
			if (steps == 0) throw new Refusal("'" + text.substring(word[0], word[1]) + "' is not a macro use CDT can expand.");
			JsonObject out = new JsonObject();
			MacroExpansionExplorer.IMacroExpansionStep full = explorer.getFullExpansion();
			out.addProperty("code", full.getCodeBeforeStep());
			out.addProperty("expanded", full.getCodeAfterStep());
			JsonArray list = new JsonArray();
			for (int i = 0; i < steps && i < 50; i++) {
				MacroExpansionExplorer.IMacroExpansionStep step = explorer.getExpansionStep(i);
				JsonObject j = new JsonObject();
				if (step.getExpandedMacro() != null) j.addProperty("macro", step.getExpandedMacro().getName());
				j.addProperty("after", step.getCodeAfterStep());
				list.add(j);
			}
			out.add("steps", list);
			return McpToolResult.success(out);
		} finally {
			index.releaseReadLock();
		}
	}
}
