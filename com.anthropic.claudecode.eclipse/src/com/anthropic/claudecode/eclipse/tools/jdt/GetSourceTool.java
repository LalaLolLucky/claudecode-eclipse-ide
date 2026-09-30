package com.anthropic.claudecode.eclipse.tools.jdt;

import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.jdt.core.IJavaElement;
import org.eclipse.jdt.core.IMember;
import org.eclipse.jdt.core.IPackageFragmentRoot;
import org.eclipse.jdt.core.ISourceRange;
import org.eclipse.jdt.core.ITypeRoot;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Returns the source of a Java type, method or field — including classes that live only
 * inside a JAR on a project's classpath (the JDK, a Maven dependency, a target-platform
 * bundle), which no file read can reach. It is what the Java editor shows on Open
 * Declaration into a library: the attached source when there is one, otherwise the
 * attached Javadoc. Adapted from JDT Bridge's {@code @source} query.
 */
public class GetSourceTool implements McpTool {

	private static final int DEFAULT_MAX_CHARS = 20_000;
	private static final int MAX_MAX_CHARS = 100_000;

	@Override
	public String toolName() {
		return "getSource";
	}

	@Override
	public String description() {
		return "Get the source code of a Java type, method or field by fully qualified name — "
				+ "including library classes inside JARs on the classpath (JDK, dependencies, "
				+ "target platform), which cannot be read as files. Falls back to the attached "
				+ "Javadoc when a library has no source attached. Examples: 'java.util.ArrayList', "
				+ "'java.util.ArrayList.add(Object)', 'org.eclipse.core.resources.IResource.DEPTH_ZERO'. "
				+ "Ask for a method rather than a whole large class to keep the result small.";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject schema = new JsonObject();
		schema.addProperty("type", "object");

		JsonObject props = new JsonObject();

		JsonObject fqn = new JsonObject();
		fqn.addProperty("type", "string");
		fqn.addProperty("description", "Fully qualified name: a type, Type.method(ParamType,...) or Type.FIELD.");
		props.add("fqn", fqn);

		JsonObject maxChars = new JsonObject();
		maxChars.addProperty("type", "integer");
		maxChars.addProperty("description",
				"Most characters of source to return (default " + DEFAULT_MAX_CHARS + ", at most " + MAX_MAX_CHARS + ").");
		props.add("maxChars", maxChars);

		schema.add("properties", props);
		JsonArray required = new JsonArray();
		required.add("fqn");
		schema.add("required", required);
		return schema;
	}

	@Override
	public McpToolResult execute(JsonObject params) {
		try {
			String fqn = params.has("fqn") ? params.get("fqn").getAsString() : null;
			if (fqn == null || fqn.isBlank()) {
				return McpToolResult.error("Missing required parameter: fqn");
			}
			int maxChars = DEFAULT_MAX_CHARS;
			if (params.has("maxChars") && !params.get("maxChars").isJsonNull()) {
				maxChars = Math.max(1, Math.min(MAX_MAX_CHARS, params.get("maxChars").getAsInt()));
			}

			IJavaElement element = JdtUtils.resolveElement(fqn);
			if (!(element instanceof IMember member)) {
				return McpToolResult.error("Could not resolve element: " + fqn
						+ ". Use a fully qualified type name, Type.method(ParamType,...) or Type.FIELD.");
			}

			JsonObject out = new JsonObject();
			out.addProperty("fqn", JdtUtils.getFqn(member));
			out.addProperty("binary", member.isBinary());
			out.addProperty("origin", origin(member));

			String source = member.getSource();
			if (source != null) {
				ISourceRange range = member.getSourceRange();
				ITypeRoot root = member.getTypeRoot();
				String whole = root != null ? root.getSource() : null;
				if (range != null && range.getOffset() >= 0 && whole != null) {
					out.addProperty("startLine", lineOf(whole, range.getOffset()));
					out.addProperty("endLine", lineOf(whole, range.getOffset() + range.getLength()));
				}
				out.addProperty("totalChars", source.length());
				if (source.length() > maxChars) {
					out.addProperty("source", source.substring(0, maxChars) + "\n…(truncated)");
					out.addProperty("note", "Truncated — ask for a single method or field for the rest.");
				} else {
					out.addProperty("source", source);
				}
			} else {
				String javadoc = member.getAttachedJavadoc(new NullProgressMonitor());
				if (javadoc != null) {
					String text = htmlToText(javadoc);
					out.addProperty("javadoc", text.length() > maxChars ? text.substring(0, maxChars) + "…" : text);
					out.addProperty("note", "No source is attached to this library; this is its attached Javadoc.");
				} else {
					out.addProperty("note", "No source or Javadoc is attached to this library. Attach "
							+ "sources to the JAR (or use a -sources artifact) to read it.");
				}
			}
			ClaudeCodeView.debug("[getSource] " + fqn + " binary=" + member.isBinary()
					+ " source=" + (source != null));
			return McpToolResult.success(out);
		} catch (Exception | LinkageError e) {
			return McpToolResult.error("getSource failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
		}
	}

	/** The file for a workspace member, the JAR or folder for a library one. */
	private static String origin(IMember member) {
		if (!member.isBinary() && member.getResource() != null && member.getResource().getLocation() != null) {
			return member.getResource().getLocation().toOSString();
		}
		IJavaElement root = member.getAncestor(IJavaElement.PACKAGE_FRAGMENT_ROOT);
		if (root instanceof IPackageFragmentRoot pfr) {
			IPath path = pfr.getPath();
			return path != null ? path.toOSString() : pfr.getElementName();
		}
		return null;
	}

	private static int lineOf(String text, int offset) {
		int line = 1;
		int end = Math.min(offset, text.length());
		for (int i = 0; i < end; i++) {
			if (text.charAt(i) == '\n') line++;
		}
		return line;
	}

	/** Attached Javadoc arrives as HTML; keep the words and line structure. */
	private static String htmlToText(String html) {
		return html.replaceAll("(?i)<br\\s*/?>|</p>|</dd>|</dt>|</li>|</h\\d>", "\n")
				.replaceAll("<[^>]+>", "")
				.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
				.replace("&quot;", "\"").replace("&nbsp;", " ").replace("&#64;", "@")
				.replaceAll("\n{3,}", "\n\n")
				.trim();
	}
}
