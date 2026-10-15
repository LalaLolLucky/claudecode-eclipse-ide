package com.anthropic.claudecode.eclipse.tools.jdt;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IWorkspaceRoot;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.core.runtime.Path;
import org.eclipse.debug.core.DebugPlugin;
import org.eclipse.debug.core.ILaunch;
import org.eclipse.debug.core.ILaunchConfiguration;
import org.eclipse.debug.core.model.IProcess;
import org.eclipse.eclemma.core.CoverageTools;
import org.eclipse.eclemma.core.ICoverageSession;
import org.eclipse.eclemma.core.ISessionListener;
import org.eclipse.eclemma.core.ISessionManager;
import org.eclipse.eclemma.core.analysis.IJavaModelCoverage;
import org.eclipse.jdt.core.ICompilationUnit;
import org.eclipse.jdt.core.IJavaElement;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.IPackageFragment;
import org.eclipse.jdt.core.IType;
import org.eclipse.jdt.core.JavaCore;
import org.jacoco.core.analysis.ICoverageNode;
import org.jacoco.core.analysis.ICounter;
import org.jacoco.core.analysis.ISourceNode;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.tools.ToolSupport;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Code coverage with EclEmma (Coverage As): run a launch configuration under coverage, and read
 * the coverage of the active session — per project and package, or a source file's lines. The
 * figures are EclEmma's, which are JaCoCo's: instructions, branches, lines and methods.
 *
 * <p>EclEmma is optional ({@code resolution:=optional}); the registry creates this tool only
 * when org.eclipse.eclemma.core and org.jacoco.core are there. Its packages are exported
 * {@code x-friends}, which the compiler marks as discouraged; the runtime does not enforce it.
 * The API used is the same in EclEmma 3.1.6 (2022) and 3.1.12 (2026).
 */
@SuppressWarnings("restriction")   // EclEmma's x-friends exports: discouraged, not enforced
public class CoverageTool implements McpTool {

	private static final List<String> ACTIONS = List.of("run", "summary", "file");

	/** How long 'run' waits for the program to end and its coverage to be read, by default and at most. */
	private static final int WAIT_SECONDS = 300;
	private static final int MOST_WAIT_SECONDS = 1800;

	/** What a session still being read is answered with. EclEmma leaves it so for good when it
	 *  could not read it (seen live: a class file deleted behind the workspace's back). */
	private static final String STILL_READING = "The coverage session is still being read; ask again in a moment. "
			+ "If it stays so, EclEmma could not read it: the errorLog tool (plugin org.eclipse.eclemma.core) says why.";

	/** How many packages 'summary' lists, those with the most lines missed first. */
	private static final int MAX_PACKAGES = 60;

	@Override
	public String toolName() {
		return "coverage";
	}

	@Override
	public String description() {
		return "Code coverage with EclEmma, as Coverage As does. action='run' runs the launch configuration "
				+ "'config' (a JUnit test or a Java application) under coverage, building first, waits for it "
				+ "to end and answers with its coverage. action='summary' gives the coverage of the active "
				+ "session: the session's projects and their packages, those with the most lines missed first, "
				+ "each with its instruction, branch, line and method counts; 'project' keeps one project. "
				+ "action='file' gives a Java source file's counts and its lines as ranges under covered, "
				+ "partlyCovered and missed; name it by 'path' (workspace path such as /Proj/src/a/B.java, or "
				+ "a full path) or 'type' (a fully qualified name).";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject props = new JsonObject();
		props.add("action", ToolSupport.choice("run, summary or file.", ACTIONS));
		props.add("config", ToolSupport.prop("string", "run: the launch configuration's name, as Run Configurations lists it."));
		props.add("waitSeconds", ToolSupport.prop("integer", "run: how long to wait for the program to end and its "
				+ "coverage to be read, 1 to " + MOST_WAIT_SECONDS + "; " + WAIT_SECONDS + " when left out."));
		props.add("project", ToolSupport.prop("string", "summary: only this project's packages."));
		props.add("path", ToolSupport.prop("string", "file: the Java file, as a workspace path or a full path."));
		props.add("type", ToolSupport.prop("string", "file: instead of 'path', a type's fully qualified name; its file is read."));
		return ToolSupport.schema(props, "action");
	}

	@Override
	public McpToolResult execute(JsonObject params) {
		ClaudeCodeView.debug("[coverage] " + params);
		try {
			String action = ToolSupport.str(params, "action");
			if (action == null || !ACTIONS.contains(action)) {
				return McpToolResult.error("'action' must be one of: " + String.join(", ", ACTIONS) + ".");
			}
			return switch (action) {
				case "run" -> run(params);
				case "summary" -> summary(ToolSupport.str(params, "project"));
				default -> file(params);
			};
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
			return McpToolResult.error("coverage was interrupted.");
		} catch (Exception | LinkageError e) {
			ClaudeCodeView.debug("[coverage] failed: " + e);
			return McpToolResult.error("coverage failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
		}
	}

	// ── run ─────────────────────────────────────────────────────────────────

	private McpToolResult run(JsonObject params) throws Exception {
		String name = ToolSupport.str(params, "config");
		if (name == null) return McpToolResult.error("'config' is required: a launch configuration's name.");
		int wait = WAIT_SECONDS;
		if (params.has("waitSeconds") && !params.get("waitSeconds").isJsonNull()) {
			wait = params.get("waitSeconds").getAsInt();
			if (wait < 1 || wait > MOST_WAIT_SECONDS) {
				return McpToolResult.error("'waitSeconds' must be from 1 to " + MOST_WAIT_SECONDS + ".");
			}
		}
		ILaunchConfiguration config = null;
		for (ILaunchConfiguration c : DebugPlugin.getDefault().getLaunchManager().getLaunchConfigurations()) {
			if (c.getName().equalsIgnoreCase(name)) { config = c; break; }
		}
		if (config == null) return McpToolResult.error("No launch configuration named '" + name + "'.");
		if (!config.supportsMode(CoverageTools.LAUNCH_MODE)) {
			return McpToolResult.error("'" + config.getName() + "' (" + config.getType().getName()
					+ ") cannot be run under coverage; EclEmma covers Java launches such as JUnit tests and Java applications.");
		}

		ISessionManager sessions = CoverageTools.getSessionManager();
		CompletableFuture<ICoverageSession> added = new CompletableFuture<>();
		String configName = config.getName();
		ISessionListener listener = new ISessionListener() {
			@Override
			public void sessionAdded(ICoverageSession s) {
				ILaunchConfiguration of = s.getLaunchConfiguration();
				if (of != null && of.getName().equals(configName)) added.complete(s);
			}

			@Override
			public void sessionRemoved(ICoverageSession s) {}

			@Override
			public void sessionActivated(ICoverageSession s) {}
		};
		long until = System.currentTimeMillis() + wait * 1000L;
		ILaunch launch;
		sessions.addSessionListener(listener);
		try {
			// build=true: launching a configuration by name skips the build otherwise.
			launch = config.launch(CoverageTools.LAUNCH_MODE, new NullProgressMonitor(), true);
			try {
				added.get(Math.max(1, until - System.currentTimeMillis()), TimeUnit.MILLISECONDS);
			} catch (TimeoutException e) {
				JsonObject late = new JsonObject();
				late.addProperty("config", configName);
				late.addProperty("finished", launch.isTerminated());
				late.addProperty("note", "Still running after " + wait + " s; its coverage is read when it ends. "
						+ "Ask action='summary' then, or stop it with the launches tool.");
				return McpToolResult.success(late);
			}
		} finally {
			sessions.removeSessionListener(listener);
		}
		IJavaModelCoverage model = analysed(until);
		JsonObject out = new JsonObject();
		out.addProperty("config", configName);
		JsonArray exits = new JsonArray();
		for (IProcess p : launch.getProcesses()) {
			if (p.isTerminated()) exits.add(p.getExitValue());
		}
		if (!exits.isEmpty()) out.add("exitValues", exits);
		if (model == null) {
			out.addProperty("note", "The program ended, but its coverage was still being read. " + STILL_READING);
			return McpToolResult.success(out);
		}
		describeSession(out, model, null);
		return McpToolResult.success(out);
	}

	/** The active session's coverage once it has been read, or null if it is not read by {@code until}. */
	private static IJavaModelCoverage analysed(long until) throws InterruptedException {
		while (true) {
			IJavaModelCoverage model = CoverageTools.getJavaModelCoverage();
			if (model != null && model != IJavaModelCoverage.LOADING) return model;
			if (System.currentTimeMillis() > until) return null;
			Thread.sleep(200);
		}
	}

	// ── summary ─────────────────────────────────────────────────────────────

	private McpToolResult summary(String project) {
		IJavaModelCoverage model = CoverageTools.getJavaModelCoverage();
		if (model == null) {
			return McpToolResult.error("There is no coverage session. Run one with action='run', or Coverage As in Eclipse.");
		}
		if (model == IJavaModelCoverage.LOADING) {
			return McpToolResult.error(STILL_READING);
		}
		JsonObject out = new JsonObject();
		describeSession(out, model, project);
		return McpToolResult.success(out);
	}

	private static void describeSession(JsonObject out, IJavaModelCoverage model, String onlyProject) {
		ICoverageSession session = CoverageTools.getSessionManager().getActiveSession();
		if (session != null) out.addProperty("session", session.getDescription());
		// The model's own counters are not filled in (seen live): the total is the projects' sum.
		int[] sum = new int[8];
		JsonArray projects = new JsonArray();
		for (IJavaProject p : model.getProjects()) {
			if (onlyProject != null && !p.getElementName().equalsIgnoreCase(onlyProject)) continue;
			ICoverageNode node = model.getCoverageFor(p);
			if (node == null) continue;
			JsonObject j = counters(node);
			j.addProperty("project", p.getElementName());
			projects.add(j);
			ICounter[] each = { node.getLineCounter(), node.getBranchCounter(), node.getInstructionCounter(), node.getMethodCounter() };
			for (int i = 0; i < each.length; i++) {
				sum[2 * i] += each[i].getCoveredCount();
				sum[2 * i + 1] += each[i].getMissedCount();
			}
		}
		JsonObject total = new JsonObject();
		total.add("lines", CoverageFormat.counter(sum[0], sum[1]));
		total.add("branches", CoverageFormat.counter(sum[2], sum[3]));
		total.add("instructions", CoverageFormat.counter(sum[4], sum[5]));
		total.add("methods", CoverageFormat.counter(sum[6], sum[7]));
		out.add("total", total);
		out.add("projects", projects);

		List<IPackageFragment> packages = new ArrayList<>();
		for (IPackageFragment f : model.getPackageFragments()) {
			if (onlyProject != null && !f.getJavaProject().getElementName().equalsIgnoreCase(onlyProject)) continue;
			ICoverageNode node = model.getCoverageFor(f);
			if (node != null && node.containsCode()) packages.add(f);
		}
		packages.sort(Comparator.comparingInt((IPackageFragment f) -> model.getCoverageFor(f).getLineCounter().getMissedCount())
				.reversed());
		JsonArray listed = new JsonArray();
		for (IPackageFragment f : packages) {
			if (listed.size() >= MAX_PACKAGES) break;
			JsonObject j = counters(model.getCoverageFor(f));
			j.addProperty("package", f.getElementName().isEmpty() ? "(default package)" : f.getElementName());
			j.addProperty("project", f.getJavaProject().getElementName());
			// Two source folders can each hold a package of one name, the default one above all.
			j.addProperty("sourceFolder", f.getParent().getPath().toString());
			listed.add(j);
		}
		out.add("packages", listed);
		if (packages.size() > listed.size()) out.addProperty("packagesNotListed", packages.size() - listed.size());
	}

	private static JsonObject counters(ICoverageNode node) {
		JsonObject j = new JsonObject();
		j.add("lines", counter(node.getLineCounter()));
		j.add("branches", counter(node.getBranchCounter()));
		j.add("instructions", counter(node.getInstructionCounter()));
		j.add("methods", counter(node.getMethodCounter()));
		return j;
	}

	private static JsonObject counter(ICounter c) {
		return CoverageFormat.counter(c.getCoveredCount(), c.getMissedCount());
	}

	// ── file ────────────────────────────────────────────────────────────────

	private McpToolResult file(JsonObject params) throws Exception {
		IJavaModelCoverage model = CoverageTools.getJavaModelCoverage();
		if (model == null) {
			return McpToolResult.error("There is no coverage session. Run one with action='run', or Coverage As in Eclipse.");
		}
		if (model == IJavaModelCoverage.LOADING) {
			return McpToolResult.error(STILL_READING);
		}
		ICompilationUnit unit = unitOf(params);
		if (unit == null) {
			return McpToolResult.error("Name a Java source file with 'path' (workspace path or full path) or 'type' "
					+ "(a fully qualified name in a source folder).");
		}
		ICoverageNode node = model.getCoverageFor(unit);
		if (node == null) {
			return McpToolResult.error(unit.getPath() + " is not in the active coverage session: its project was not "
					+ "in the launch's scope, or no class of it was loaded.");
		}
		JsonObject out = counters(node);
		out.addProperty("file", unit.getPath().toString());
		if (node instanceof ISourceNode source && source.getFirstLine() != ISourceNode.UNKNOWN_LINE) {
			int first = source.getFirstLine(), last = source.getLastLine();
			int[] status = new int[last - first + 1];
			for (int line = first; line <= last; line++) status[line - first] = source.getLine(line).getStatus();
			out.add("lineStatus", CoverageFormat.lines(first, status));
		}
		return McpToolResult.success(out);
	}

	private static ICompilationUnit unitOf(JsonObject params) {
		String type = ToolSupport.str(params, "type");
		if (type != null) {
			IType t = JdtUtils.findType(type);
			return t != null ? t.getCompilationUnit() : null;
		}
		String path = ToolSupport.str(params, "path");
		if (path == null) return null;
		IWorkspaceRoot root = ResourcesPlugin.getWorkspace().getRoot();
		IFile file = null;
		java.nio.file.Path full = null;
		try {
			full = java.nio.file.Paths.get(path);
		} catch (RuntimeException notAPath) {
			// a workspace path, read below
		}
		if (full != null && full.isAbsolute() && java.nio.file.Files.exists(full)) {
			IFile[] found = root.findFilesForLocationURI(full.toUri());
			if (found.length > 0) file = found[0];
		}
		if (file == null) {
			try {
				IFile inWorkspace = root.getFile(new Path(path));
				if (inWorkspace.exists()) file = inWorkspace;
			} catch (IllegalArgumentException notInTheWorkspace) {
				// a full path that is not there, or a single segment
			}
		}
		if (file == null) return null;
		IJavaElement element = JavaCore.create(file);
		return element instanceof ICompilationUnit unit ? unit : null;
	}
}
