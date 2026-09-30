package com.anthropic.claudecode.eclipse.mcp;

import java.util.LinkedHashMap;
import java.util.Map;

import org.eclipse.core.runtime.Platform;

import com.anthropic.claudecode.eclipse.Activator;
import com.anthropic.claudecode.eclipse.tools.AcceptDiffTool;
import com.anthropic.claudecode.eclipse.tools.BuildTool;
import com.anthropic.claudecode.eclipse.tools.CheckDocumentDirtyTool;
import com.anthropic.claudecode.eclipse.tools.CleanTool;
import com.anthropic.claudecode.eclipse.tools.CloseAllDiffTabsTool;
import com.anthropic.claudecode.eclipse.tools.DebugTool;
import com.anthropic.claudecode.eclipse.tools.ErrorLogTool;
import com.anthropic.claudecode.eclipse.tools.FormatTool;
import com.anthropic.claudecode.eclipse.tools.GetCurrentSelectionTool;
import com.anthropic.claudecode.eclipse.tools.GetDiagnosticsTool;
import com.anthropic.claudecode.eclipse.tools.GetLatestSelectionTool;
import com.anthropic.claudecode.eclipse.tools.GetOpenEditorsTool;
import com.anthropic.claudecode.eclipse.tools.GetWorkspaceFoldersTool;
import com.anthropic.claudecode.eclipse.tools.LaunchesTool;
import com.anthropic.claudecode.eclipse.tools.OpenDiffTool;
import com.anthropic.claudecode.eclipse.tools.OpenFileTool;
import com.anthropic.claudecode.eclipse.tools.RefactorResourceTool;
import com.anthropic.claudecode.eclipse.tools.RefreshTool;
import com.anthropic.claudecode.eclipse.tools.RejectDiffTool;
import com.anthropic.claudecode.eclipse.tools.RunAsTool;
import com.anthropic.claudecode.eclipse.tools.SaveDocumentTool;
import com.anthropic.claudecode.eclipse.tools.jdt.FindReferencesTool;
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
        // FormatTool the platform debug model and editor bundles, and RefactorResourceTool
        // org.eclipse.ltk.core.refactoring — all hard Require-Bundles in the base Platform.
        // RunAsTool reaches the individual launchers (PDE's "Eclipse Application", JDT's "Java
        // Application", …) through the extension registry, and DebugTool each language's
        // debugger through the debug model's interfaces, never an import — so a missing
        // launcher or debugger is an absent option rather than a class-loading failure.
        register(new RefreshTool());
        register(new CleanTool());
        register(new BuildTool());
        register(new RunAsTool());
        register(new LaunchesTool());
        register(new ErrorLogTool());
        register(new DebugTool());
        register(new FormatTool());
        register(new RefactorResourceTool());
        register(new com.anthropic.claudecode.eclipse.tools.ApprovalPromptTool());
        register(new com.anthropic.claudecode.eclipse.tools.AskUserQuestionTool());
        registerJdtToolsIfAvailable();
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
