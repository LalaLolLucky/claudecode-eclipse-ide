package com.anthropic.claudecode.eclipse.mcp;

import java.util.LinkedHashMap;
import java.util.Map;

import org.eclipse.core.runtime.Platform;

import com.anthropic.claudecode.eclipse.Activator;
import com.anthropic.claudecode.eclipse.tools.AcceptDiffTool;
import com.anthropic.claudecode.eclipse.tools.AutoBuildTool;
import com.anthropic.claudecode.eclipse.tools.BuildTool;
import com.anthropic.claudecode.eclipse.tools.CheckDocumentDirtyTool;
import com.anthropic.claudecode.eclipse.tools.ClaudeCodeEclipseTool;
import com.anthropic.claudecode.eclipse.tools.CleanTool;
import com.anthropic.claudecode.eclipse.tools.CloseAllDiffTabsTool;
import com.anthropic.claudecode.eclipse.tools.DebugTool;
import com.anthropic.claudecode.eclipse.tools.EclipseCustomizePerspectiveTool;
import com.anthropic.claudecode.eclipse.tools.EclipseDialogTool;
import com.anthropic.claudecode.eclipse.tools.EclipseOpenPerspectiveTool;
import com.anthropic.claudecode.eclipse.tools.EclipseShowViewTool;
import com.anthropic.claudecode.eclipse.tools.ErrorLogTool;
import com.anthropic.claudecode.eclipse.tools.FormatTool;
import com.anthropic.claudecode.eclipse.tools.ForgeTool;
import com.anthropic.claudecode.eclipse.tools.GetCurrentSelectionTool;
import com.anthropic.claudecode.eclipse.tools.GetDiagnosticsTool;
import com.anthropic.claudecode.eclipse.tools.GetLatestSelectionTool;
import com.anthropic.claudecode.eclipse.tools.GetOpenEditorsTool;
import com.anthropic.claudecode.eclipse.tools.GetWorkspaceFoldersTool;
import com.anthropic.claudecode.eclipse.tools.LaunchesTool;
import com.anthropic.claudecode.eclipse.tools.LocalHistoryTool;
import com.anthropic.claudecode.eclipse.tools.MavenUpdateTool;
import com.anthropic.claudecode.eclipse.tools.OpenDiffTool;
import com.anthropic.claudecode.eclipse.tools.OpenFileTool;
import com.anthropic.claudecode.eclipse.tools.ProjectsTool;
import com.anthropic.claudecode.eclipse.tools.QuickFixTool;
import com.anthropic.claudecode.eclipse.tools.RefactorResourceTool;
import com.anthropic.claudecode.eclipse.tools.RefreshTool;
import com.anthropic.claudecode.eclipse.tools.RejectDiffTool;
import com.anthropic.claudecode.eclipse.tools.RunAsTool;
import com.anthropic.claudecode.eclipse.tools.SaveDocumentTool;
import com.anthropic.claudecode.eclipse.tools.TargetPlatformTool;
import com.anthropic.claudecode.eclipse.tools.TasksTool;
import com.anthropic.claudecode.eclipse.tools.ToolSupport;
import com.anthropic.claudecode.eclipse.tools.WorkspaceSearchTool;
import com.anthropic.claudecode.eclipse.tools.jdt.CallHierarchyTool;
import com.anthropic.claudecode.eclipse.tools.jdt.ClasspathTool;
import com.anthropic.claudecode.eclipse.tools.jdt.FindReferencesTool;
import com.anthropic.claudecode.eclipse.tools.jdt.GenerateJavaTool;
import com.anthropic.claudecode.eclipse.tools.jdt.GetSourceTool;
import com.anthropic.claudecode.eclipse.tools.jdt.GetSymbolInfoTool;
import com.anthropic.claudecode.eclipse.tools.jdt.GetTypeHierarchyTool;
import com.anthropic.claudecode.eclipse.tools.jdt.HotCodeReplaceMonitor;
import com.anthropic.claudecode.eclipse.tools.jdt.OrganizeImportsTool;
import com.anthropic.claudecode.eclipse.tools.jdt.RefactorJavaTool;
import com.anthropic.claudecode.eclipse.tools.jdt.RunTestsTool;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

public class McpToolRegistry {

    private final Map<String, McpTool> tools = new LinkedHashMap<>();

    public McpToolRegistry() {
        register(new OpenFileTool());
        register(new OpenDiffTool());
        register(new AcceptDiffTool());
        register(new RejectDiffTool());
        register(new GetCurrentSelectionTool());
        register(new GetLatestSelectionTool());
        register(new GetOpenEditorsTool());
        register(new GetWorkspaceFoldersTool());
        register(new CheckDocumentDirtyTool());
        register(new SaveDocumentTool());
        register(new GetDiagnosticsTool());
        register(new CloseAllDiffTabsTool());
        // Unconditional: BuildTool needs only org.eclipse.core.resources, RunAsTool only
        // org.eclipse.debug.{core,ui}, LaunchesTool those plus org.eclipse.ui.console and
        // org.eclipse.jface.text, ErrorLogTool only org.eclipse.core.runtime, DebugTool and
        // FormatTool the platform debug model and editor bundles, RefactorResourceTool
        // org.eclipse.ltk.core.refactoring, and EclipseShowViewTool,
        // EclipseOpenPerspectiveTool, EclipseCustomizePerspectiveTool and EclipseDialogTool only
        // org.eclipse.ui (EclipseCustomizePerspectiveTool also its internal workbench API, guarded,
        // and org.eclipse.e4.ui.model.workbench for the window's toolbar) — all hard Require-Bundles in the base
        // Platform. ForgeTool, AutoBuildTool, ProjectsTool, LocalHistoryTool and TasksTool need only
        // org.eclipse.core.resources, and QuickFixTool org.eclipse.ui.ide.
        // RunAsTool reaches the individual launchers (PDE's "Eclipse Application", JDT's "Java
        // Application", …) through the extension registry, and DebugTool each language's
        // debugger through the debug model's interfaces, never an import — so a missing
        // launcher or debugger is an absent option rather than a class-loading failure.
        register(new RefreshTool());
        register(new CleanTool());
        register(new BuildTool());
        register(new ForgeTool());
        register(new AutoBuildTool());
        register(new RunAsTool());
        register(new LaunchesTool());
        register(new ErrorLogTool());
        register(new DebugTool());
        register(new FormatTool());
        register(new RefactorResourceTool());
        register(new QuickFixTool());
        register(new ProjectsTool());
        register(new LocalHistoryTool());
        register(new TasksTool());
        register(new EclipseShowViewTool());
        register(new EclipseOpenPerspectiveTool());
        // Built on the workbench's internal API: an Eclipse without it loses this tool, not all of them.
        try {
            register(new EclipseCustomizePerspectiveTool());
        } catch (LinkageError | RuntimeException e) {
            Activator.logError("eclipseCustomizePerspective could not be created; skipping", e);
        }
        register(new EclipseDialogTool());
        register(new ClaudeCodeEclipseTool());
        register(new com.anthropic.claudecode.eclipse.tools.ApprovalPromptTool());
        register(new com.anthropic.claudecode.eclipse.tools.AskUserQuestionTool());
        registerOptionalToolsIfAvailable();
        registerJdtToolsIfAvailable();
    }

    /**
     * Registers the tools that drive a plug-in an Eclipse may not have. workspaceSearch
     * imports the File Search engine, an optional {@code Require-Bundle}, so its class is
     * loaded only behind the guard, like the JDT tools below. targetPlatform and mavenUpdate
     * reach PDE and m2e by name and import neither; their guards only keep a tool that could
     * do nothing out of the list.
     */
    private void registerOptionalToolsIfAvailable() {
        if (ToolSupport.installed("org.eclipse.search")) {
            try {
                register(new WorkspaceSearchTool());
            } catch (LinkageError | RuntimeException e) {
                Activator.logError("File Search present but the workspaceSearch tool could not be created; skipping", e);
            }
        }
        // cIndex reads CDT's index: org.eclipse.cdt.core is an optional Require-Bundle, so its
        // class is loaded only behind this guard.
        if (ToolSupport.installed("org.eclipse.cdt.core")) {
            try {
                register(new com.anthropic.claudecode.eclipse.tools.cdt.CIndexTool());
            } catch (LinkageError | RuntimeException e) {
                Activator.logError("CDT present but the cIndex tool could not be created; skipping", e);
            }
        }
        // pydevCode drives PyDev's own Go to Definition / Find References: the four optional bundles
        // the Python entity resolver checks (EntitiesRegistry), and the parser for the found nodes.
        if (ToolSupport.installed("org.python.pydev") && ToolSupport.installed("org.python.pydev.core")
                && ToolSupport.installed("org.python.pydev.ast") && ToolSupport.installed("com.python.pydev.analysis")
                && ToolSupport.installed("org.python.pydev.parser")) {
            try {
                register(new com.anthropic.claudecode.eclipse.tools.pydev.PyDevCodeTool());
            } catch (LinkageError | RuntimeException e) {
                Activator.logError("PyDev present but the pydevCode tool could not be created; skipping", e);
            }
        }
        if (ToolSupport.installed(TargetPlatformTool.PDE)) register(new TargetPlatformTool());
        if (ToolSupport.installed(MavenUpdateTool.M2E)) register(new MavenUpdateTool());
    }

    /**
     * Registers the Java-semantic tools only when JDT is installed. Eclipse is used for many
     * languages (CDT, PyDev, …) and JDT may be absent, so the JDT bundles are optional
     * ({@code resolution:=optional}) and these tools are skipped rather than failing the whole
     * plugin. The classes that reference JDT types are loaded lazily — only the {@code new ...()}
     * below triggers loading — so the {@link Platform#getBundle} guards keep them off the
     * classloader entirely when JDT is missing. The {@link LinkageError} catch covers the case
     * where JDT is present but version-incompatible.
     */
    private void registerJdtToolsIfAvailable() {
        if (Platform.getBundle("org.eclipse.jdt.core") == null) {
            return; // No Java tooling — leave the Java semantic tools unregistered.
        }
        try {
            register(new FindReferencesTool());
            register(new GetTypeHierarchyTool());
            register(new GetSymbolInfoTool());
            register(new GetSourceTool());
            register(new CallHierarchyTool());
            register(new ClasspathTool());
            register(new GenerateJavaTool());
        } catch (LinkageError | RuntimeException e) {
            Activator.logError("JDT present but Java navigation tools could not be created; skipping", e);
        }

        // refactorJava and organizeImports need org.eclipse.jdt.core.manipulation, where JDT's
        // refactoring descriptors and OrganizeImportsOperation live. It ships with JDT, but is
        // checked on its own like the JUnit bundles below.
        if (Platform.getBundle("org.eclipse.jdt.core.manipulation") != null) {
            try {
                register(new RefactorJavaTool());
                register(new OrganizeImportsTool());
            } catch (LinkageError | RuntimeException e) {
                Activator.logError("JDT manipulation present but Java refactoring tools could not be created; skipping", e);
            }
        }

        // Hot code replace outcomes, reported by the debug tool, come from org.eclipse.jdt.debug.
        if (Platform.getBundle("org.eclipse.jdt.debug") != null) {
            try {
                HotCodeReplaceMonitor.install();
            } catch (LinkageError | RuntimeException e) {
                Activator.logError("JDT debug present but the hot code replace monitor could not be installed; skipping", e);
            }
        }

        // coverage drives EclEmma (org.eclipse.eclemma.core) and reads JaCoCo's figures
        // (org.jacoco.core), both optional and checked on their own.
        if (Platform.getBundle("org.eclipse.eclemma.core") != null && Platform.getBundle("org.jacoco.core") != null) {
            try {
                register(new com.anthropic.claudecode.eclipse.tools.jdt.CoverageTool());
            } catch (LinkageError | RuntimeException e) {
                Activator.logError("EclEmma present but the coverage tool could not be created; skipping", e);
            }
        }

        // runTests additionally needs the JUnit + launching tooling, which can be absent even
        // when jdt.core is present (e.g. a JDT install without the JUnit feature).
        if (Platform.getBundle("org.eclipse.jdt.junit.core") == null
                || Platform.getBundle("org.eclipse.jdt.launching") == null) {
            return;
        }
        try {
            register(new RunTestsTool());
        } catch (LinkageError | RuntimeException e) {
            Activator.logError("JDT JUnit tooling present but runTests tool could not be created; skipping", e);
        }
    }

    private void register(McpTool tool) {
        tools.put(tool.toolName(), tool);
    }

    public McpTool getTool(String name) {
        return tools.get(name);
    }

    public JsonArray getToolDefinitions() {
        JsonArray defs = new JsonArray();
        for (McpTool tool : tools.values()) {
            JsonObject def = new JsonObject();
            def.addProperty("name", tool.toolName());
            def.addProperty("description", tool.description());
            def.add("inputSchema", tool.inputSchema());
            defs.add(def);
        }
        return defs;
    }

    public int getToolCount() {
        return tools.size();
    }
}
