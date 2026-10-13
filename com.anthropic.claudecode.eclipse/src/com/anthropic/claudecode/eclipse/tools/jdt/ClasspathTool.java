package com.anthropic.claudecode.eclipse.tools.jdt;

import org.eclipse.core.resources.IMarker;
import org.eclipse.core.resources.IProject;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.jdt.core.IClasspathContainer;
import org.eclipse.jdt.core.IClasspathEntry;
import org.eclipse.jdt.core.IJavaModelMarker;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.JavaCore;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.anthropic.claudecode.eclipse.tools.ToolSupport;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * A Java project's build path (Properties &gt; Java Build Path): its source folders, the
 * projects and libraries it compiles against, and what is wrong with it. It answers "why
 * can this type not be resolved" from what the compiler was given, which a {@code .classpath}
 * file only half says: the containers in it (the JRE, plug-in dependencies, Maven
 * dependencies) are filled in by other plug-ins.
 */
public class ClasspathTool implements McpTool {

	/** Cap on the entries listed, so a project with a very long build path can't flood the response. */
	private static final int MAX_ENTRIES = 500;

	@Override
	public String toolName() {
		return "classpath";
	}

	@Override
	public String description() {
		return "Show a Java project's build path: source folders, required projects, libraries, and "
				+ "the containers other plug-ins fill in (JRE, plug-in dependencies, Maven "
				+ "dependencies), with the compiler level, the output folder and any build path "
				+ "problems. By default a container is one entry that says how many libraries it "
				+ "holds; with 'resolved' true, every entry is listed as the compiler sees it, each "
				+ "container replaced by its libraries. Use it when a type or import cannot be resolved.";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject props = new JsonObject();
		props.add("project", ToolSupport.prop("string", "The Java project's name."));
		props.add("resolved", ToolSupport.prop("boolean", "true lists the libraries inside each container "
				+ "instead of the container (default false)."));
		return ToolSupport.schema(props, "project");
	}

	@Override
	public McpToolResult execute(JsonObject params) {
		ClaudeCodeView.debug("[classpath] " + params);
		try {
			String name = ToolSupport.str(params, "project");
			if (name == null) return McpToolResult.error("Missing required parameter: project");
			IProject project = ResourcesPlugin.getWorkspace().getRoot().getProject(name);
			if (!project.isAccessible()) return McpToolResult.error("No open project named " + name + ".");
			if (!project.hasNature(JavaCore.NATURE_ID)) return McpToolResult.error(name + " is not a Java project.");
			IJavaProject java = JavaCore.create(project);
			boolean resolved = ToolSupport.bool(params, "resolved", false);

			JsonObject result = new JsonObject();
			result.addProperty("project", name);
			result.addProperty("compliance", java.getOption(JavaCore.COMPILER_COMPLIANCE, true));
			result.addProperty("outputFolder", java.getOutputLocation().toString());

			IClasspathEntry[] all = resolved ? java.getResolvedClasspath(true) : java.getRawClasspath();
			JsonArray entries = new JsonArray();
			for (IClasspathEntry entry : all) {
				if (entries.size() >= MAX_ENTRIES) break;
				entries.add(describe(entry, java));
			}
			result.addProperty("entryCount", all.length);
			result.add("entries", entries);
			if (all.length > entries.size()) result.addProperty("truncated", true);

			JsonArray problems = new JsonArray();
			for (IMarker m : project.findMarkers(IJavaModelMarker.BUILDPATH_PROBLEM_MARKER, true, IResource.DEPTH_ZERO)) {
				problems.add(m.getAttribute(IMarker.MESSAGE, ""));
			}
			if (!problems.isEmpty()) result.add("problems", problems);
			return McpToolResult.success(result);
		} catch (Exception | LinkageError e) {
			ClaudeCodeView.debug("[classpath] failed: " + e);
			return McpToolResult.error("classpath failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
		}
	}

	private static JsonObject describe(IClasspathEntry entry, IJavaProject java) {
		JsonObject j = new JsonObject();
		j.addProperty("kind", switch (entry.getEntryKind()) {
			case IClasspathEntry.CPE_SOURCE -> "source";
			case IClasspathEntry.CPE_LIBRARY -> "library";
			case IClasspathEntry.CPE_PROJECT -> "project";
			case IClasspathEntry.CPE_VARIABLE -> "variable";
			default -> "container";
		});
		j.addProperty("path", entry.getPath().toString());
		if (entry.getOutputLocation() != null) j.addProperty("outputFolder", entry.getOutputLocation().toString());
		if (entry.getSourceAttachmentPath() != null) j.addProperty("sources", entry.getSourceAttachmentPath().toString());
		if (entry.isExported()) j.addProperty("exported", true);
		if (entry.isTest()) j.addProperty("test", true);
		if (entry.getEntryKind() == IClasspathEntry.CPE_CONTAINER) {
			try {
				IClasspathContainer container = JavaCore.getClasspathContainer(entry.getPath(), java);
				if (container == null) {
					j.addProperty("unbound", true);   // nothing fills it in: its plug-in is missing or failed
				} else {
					j.addProperty("description", container.getDescription());
					j.addProperty("libraryCount", container.getClasspathEntries().length);
				}
			} catch (Exception e) {
				j.addProperty("unbound", true);
			}
		}
		return j;
	}
}
