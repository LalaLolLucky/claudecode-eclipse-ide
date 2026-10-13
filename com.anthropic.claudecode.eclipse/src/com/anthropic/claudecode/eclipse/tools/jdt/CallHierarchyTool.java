package com.anthropic.claudecode.eclipse.tools.jdt;

import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.eclipse.core.runtime.CoreException;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.jdt.core.IJavaElement;
import org.eclipse.jdt.core.IMember;
import org.eclipse.jdt.core.IMethod;
import org.eclipse.jdt.core.ISourceRange;
import org.eclipse.jdt.core.IType;
import org.eclipse.jdt.core.ITypeRoot;
import org.eclipse.jdt.core.dom.AST;
import org.eclipse.jdt.core.dom.ASTNode;
import org.eclipse.jdt.core.dom.ASTParser;
import org.eclipse.jdt.core.dom.ASTVisitor;
import org.eclipse.jdt.core.dom.ClassInstanceCreation;
import org.eclipse.jdt.core.dom.CompilationUnit;
import org.eclipse.jdt.core.dom.ConstructorInvocation;
import org.eclipse.jdt.core.dom.IMethodBinding;
import org.eclipse.jdt.core.dom.MethodInvocation;
import org.eclipse.jdt.core.dom.NodeFinder;
import org.eclipse.jdt.core.dom.SuperConstructorInvocation;
import org.eclipse.jdt.core.dom.SuperMethodInvocation;
import org.eclipse.jdt.core.search.IJavaSearchConstants;
import org.eclipse.jdt.core.search.SearchEngine;
import org.eclipse.jdt.core.search.SearchMatch;
import org.eclipse.jdt.core.search.SearchParticipant;
import org.eclipse.jdt.core.search.SearchPattern;
import org.eclipse.jdt.core.search.SearchRequestor;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.anthropic.claudecode.eclipse.tools.ToolSupport;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * The call hierarchy of a Java method (Open Call Hierarchy): what calls it, and what calls
 * those, or what it calls, and what those call — a tree, where {@link FindReferencesTool}
 * gives one flat level.
 *
 * <p>Built on JDT's public API only: callers come from the search engine, grouped by the
 * member each reference is in; callees from the method's syntax tree with bindings
 * resolved. JDT's own call hierarchy classes are internal and are not used.
 */
public class CallHierarchyTool implements McpTool {

	private static final List<String> DIRECTIONS = List.of("callers", "callees");
	private static final int MAX_DEPTH = 5;
	private static final int LISTED = 200;
	private static final int MOST_LISTED = 1000;

	@Override
	public String toolName() {
		return "callHierarchy";
	}

	@Override
	public String description() {
		return "Show the call hierarchy of a Java method as a tree. direction='callers' (default): "
				+ "the methods, initializers and fields that call it, and what calls those; "
				+ "direction='callees': the methods and constructors it calls, and what those call. "
				+ "'depth' says how many levels, 1 to " + MAX_DEPTH + " (default 2). Each entry has the "
				+ "lines of the calls. Input: the method's fully qualified name with its parameter "
				+ "types, e.g. 'com.example.Foo.bar(String,int)'; a constructor is named like its "
				+ "type, 'com.example.Foo.Foo(int)'. A call made through an interface or a "
				+ "superclass counts for the method declared there, not for the ones overriding it.";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject props = new JsonObject();
		props.add("fqn", ToolSupport.prop("string", "Fully qualified name of the method, with parameter types."));
		props.add("direction", ToolSupport.choice("callers (default) or callees.", DIRECTIONS));
		props.add("depth", ToolSupport.prop("integer", "Levels to follow, 1 to " + MAX_DEPTH + " (default 2)."));
		props.add("limit", ToolSupport.prop("integer", "Most entries in the whole tree, 1 to " + MOST_LISTED
				+ " (default " + LISTED + ")."));
		return ToolSupport.schema(props, "fqn");
	}

	@Override
	public McpToolResult execute(JsonObject params) {
		ClaudeCodeView.debug("[callHierarchy] " + params);
		try {
			String fqn = ToolSupport.str(params, "fqn");
			if (fqn == null) return McpToolResult.error("Missing required parameter: fqn");
			String direction = ToolSupport.str(params, "direction");
			if (direction == null) direction = "callers";
			if (!DIRECTIONS.contains(direction)) {
				return McpToolResult.error("Unknown direction: " + direction + ". Use callers or callees.");
			}
			if (!(JdtUtils.resolveElement(fqn) instanceof IMethod method)) {
				return McpToolResult.error("Could not resolve a method named " + fqn + ". Give its parameter "
						+ "types, e.g. 'com.example.Foo.bar(String,int)'.");
			}
			int depth = ToolSupport.integer(params, "depth", 2, 1, MAX_DEPTH);
			Walk walk = new Walk(ToolSupport.integer(params, "limit", LISTED, 1, MOST_LISTED));

			Set<String> path = new HashSet<>();
			path.add(label(method));
			JsonObject result = new JsonObject();
			result.addProperty("method", label(method));
			if (direction.equals("callers")) {
				result.add("callers", walk.callers(method, depth, path));
			} else {
				if (walk.syntaxOf(method) == null) {
					return McpToolResult.error("No source is available for " + label(method)
							+ ", so what it calls cannot be read.");
				}
				result.add("callees", walk.callees(method, depth, path));
			}
			result.addProperty("entries", walk.listed);
			if (walk.truncated) {
				result.addProperty("truncated", true);
				result.addProperty("note", "The tree has more entries than the " + walk.limit + " listed.");
			}
			return McpToolResult.success(result);
		} catch (Exception | LinkageError e) {
			ClaudeCodeView.debug("[callHierarchy] failed: " + e);
			return McpToolResult.error("callHierarchy failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
		}
	}

	/** One call's walk of the tree: how much it has listed, and the files it has read. */
	private static final class Walk {

		final int limit;
		int listed;
		boolean truncated;

		private final Map<ITypeRoot, CompilationUnit> syntax = new HashMap<>();
		private final Map<ITypeRoot, String> sources = new HashMap<>();

		Walk(int limit) {
			this.limit = limit;
		}

		JsonArray callers(IMethod method, int depth, Set<String> path) throws CoreException {
			Map<String, IMember> members = new LinkedHashMap<>();
			Map<String, JsonArray> lines = new HashMap<>();
			SearchPattern pattern = SearchPattern.createPattern(method, IJavaSearchConstants.REFERENCES);
			new SearchEngine().search(pattern, new SearchParticipant[] { SearchEngine.getDefaultSearchParticipant() },
					SearchEngine.createWorkspaceScope(), new SearchRequestor() {
						@Override
						public void acceptSearchMatch(SearchMatch match) {
							if (match.isInsideDocComment() || !(match.getElement() instanceof IMember member)) return;
							String key = label(member);
							members.putIfAbsent(key, member);
							int line = lineOf(member.getTypeRoot(), match.getOffset());
							if (line > 0) lines.computeIfAbsent(key, k -> new JsonArray()).add(line);
						}
					}, new NullProgressMonitor());

			JsonArray out = new JsonArray();
			for (Map.Entry<String, IMember> e : members.entrySet()) {
				if (!take()) break;
				JsonObject node = describe(e.getKey(), e.getValue(), lines.get(e.getKey()));
				// A method already on the way down is a recursion: listed, not followed again.
				if (depth > 1 && e.getValue() instanceof IMethod caller && path.add(e.getKey())) {
					node.add("callers", callers(caller, depth - 1, path));
					path.remove(e.getKey());
				}
				out.add(node);
			}
			return out;
		}

		JsonArray callees(IMethod method, int depth, Set<String> path) throws CoreException {
			JsonArray out = new JsonArray();
			CompilationUnit ast = syntaxOf(method);
			if (ast == null) return out;
			ISourceRange range = method.getSourceRange();
			ASTNode body = NodeFinder.perform(ast, range.getOffset(), range.getLength());
			if (body == null) return out;

			Map<String, IMethod> called = new LinkedHashMap<>();
			Map<String, JsonArray> lines = new HashMap<>();
			body.accept(new ASTVisitor() {
				private void add(IMethodBinding binding, ASTNode call) {
					if (binding == null) return;
					IMethodBinding declared = binding.getMethodDeclaration();
					IJavaElement element = declared.getJavaElement();
					// A constructor nobody wrote has no element; it is named from its binding.
					String key = element instanceof IMethod m ? label(m)
							: declared.getDeclaringClass().getQualifiedName() + "." + declared.getName() + "()";
					called.putIfAbsent(key, element instanceof IMethod m ? m : null);
					lines.computeIfAbsent(key, k -> new JsonArray()).add(ast.getLineNumber(call.getStartPosition()));
				}

				@Override
				public boolean visit(MethodInvocation node) {
					add(node.resolveMethodBinding(), node);
					return true;
				}

				@Override
				public boolean visit(SuperMethodInvocation node) {
					add(node.resolveMethodBinding(), node);
					return true;
				}

				@Override
				public boolean visit(ClassInstanceCreation node) {
					add(node.resolveConstructorBinding(), node);
					return true;
				}

				@Override
				public boolean visit(ConstructorInvocation node) {
					add(node.resolveConstructorBinding(), node);
					return true;
				}

				@Override
				public boolean visit(SuperConstructorInvocation node) {
					add(node.resolveConstructorBinding(), node);
					return true;
				}
			});

			for (Map.Entry<String, IMethod> e : called.entrySet()) {
				if (!take()) break;
				JsonObject node = describe(e.getKey(), e.getValue(), lines.get(e.getKey()));
				IMethod callee = e.getValue();
				// Followed only into source of the workspace: a library method's calls are its own business.
				if (depth > 1 && callee != null && callee.getCompilationUnit() != null && path.add(e.getKey())) {
					node.add("callees", callees(callee, depth - 1, path));
					path.remove(e.getKey());
				}
				out.add(node);
			}
			return out;
		}

		/** Counts one more entry, or says there is no room for it. */
		private boolean take() {
			if (listed >= limit) {
				truncated = true;
				return false;
			}
			listed++;
			return true;
		}

		/** The parsed file a method is in, with bindings; null when it has no source. */
		CompilationUnit syntaxOf(IMethod method) throws CoreException {
			ITypeRoot root = method.getTypeRoot();
			if (root == null || method.getSourceRange() == null || method.getSourceRange().getLength() <= 0) return null;
			if (syntax.containsKey(root)) return syntax.get(root);
			CompilationUnit ast = null;
			if (root.getSource() != null) {
				ASTParser parser = ASTParser.newParser(AST.getJLSLatest());
				parser.setSource(root);
				parser.setResolveBindings(true);
				ast = (CompilationUnit) parser.createAST(new NullProgressMonitor());
			}
			syntax.put(root, ast);
			return ast;
		}

		private int lineOf(ITypeRoot root, int offset) {
			if (root == null || offset < 0) return 0;
			String source = sources.computeIfAbsent(root, r -> {
				try {
					String s = r.getSource();
					return s == null ? "" : s;
				} catch (CoreException e) {
					return "";
				}
			});
			if (offset > source.length()) return 0;
			int line = 1;
			for (int i = 0; i < offset; i++) {
				if (source.charAt(i) == '\n') line++;
			}
			return line;
		}
	}

	private static JsonObject describe(String label, IMember member, JsonArray lines) {
		JsonObject node = new JsonObject();
		node.addProperty("element", label);
		if (member != null && member.getResource() != null && member.getResource().getLocation() != null) {
			node.addProperty("file", member.getResource().getLocation().toOSString());
		} else if (member != null && member.isBinary()) {
			node.addProperty("library", true);
		}
		if (lines != null) node.add("callLines", lines);
		return node;
	}

	/** A member's name as these tools spell one; an initializer has none of its own. */
	private static String label(IMember member) {
		if (member instanceof IMethod || member instanceof IType || member.getElementType() == IJavaElement.FIELD) {
			return JdtUtils.getFqn(member);
		}
		IType declaring = member.getDeclaringType();
		return (declaring == null ? "" : declaring.getFullyQualifiedName() + ".") + "<initializer>";
	}
}
