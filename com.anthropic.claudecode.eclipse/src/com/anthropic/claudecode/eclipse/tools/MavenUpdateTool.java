package com.anthropic.claudecode.eclipse.tools;

import java.util.ArrayList;
import java.util.List;

import org.eclipse.core.resources.IProject;
import org.eclipse.core.runtime.NullProgressMonitor;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Maven &gt; Update Project for Maven projects: makes Eclipse's project settings — source
 * folders, compiler level, dependencies on the build path — follow the {@code pom.xml}
 * again after it has changed.
 *
 * <p>m2e is reached by name, through {@link ToolSupport#call}, and never imported, for the
 * reason given at {@link TargetPlatformTool}. The one method called,
 * {@code IProjectConfigurationManager.updateProjectConfiguration(IProject, IProgressMonitor)},
 * is API in m2e 1.x and 2.x alike.
 */
public class MavenUpdateTool implements McpTool {

    public static final String M2E = "org.eclipse.m2e.core";

    private static final String MAVEN_NATURE = "org.eclipse.m2e.core.maven2Nature";

    @Override
    public String toolName() {
        return "mavenUpdate";
    }

    @Override
    public String description() {
        return "Update Maven projects from their pom.xml (Maven > Update Project): Eclipse's source "
                + "folders, compiler settings and build path are made to follow the pom again. Run it "
                + "after a pom.xml has changed, then build. Scope: the named projects, or every open "
                + "Maven project when none are given. It can download dependencies and take a while.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject props = new JsonObject();
        props.add("projects", ToolSupport.strings(
                "Maven project names to update. Omit or leave empty for every open Maven project."));
        return ToolSupport.schema(props);
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        ClaudeCodeView.debug("[mavenUpdate] " + params);
        try {
            boolean named = !ToolSupport.list(params, "projects").isEmpty();
            List<IProject> targets = new ArrayList<>();
            List<String> notMaven = new ArrayList<>();
            for (IProject p : ToolSupport.projects(params, "update")) {
                if (p.hasNature(MAVEN_NATURE)) targets.add(p);
                else if (named) notMaven.add(p.getName());
            }
            if (!notMaven.isEmpty()) {
                return McpToolResult.error("Not a Maven project in Eclipse: " + String.join(", ", notMaven));
            }
            if (targets.isEmpty()) return McpToolResult.error("The workspace has no open Maven projects.");

            Object manager = ToolSupport.call(ToolSupport.classOf(M2E, "org.eclipse.m2e.core.MavenPlugin"), null,
                    "getProjectConfigurationManager");
            Class<?> managerType = ToolSupport.classOf(M2E, "org.eclipse.m2e.core.project.IProjectConfigurationManager");

            long started = System.currentTimeMillis();
            JsonArray updated = new JsonArray();
            JsonArray failed = new JsonArray();
            for (IProject p : targets) {
                try {
                    ToolSupport.call(managerType, manager, "updateProjectConfiguration", p, new NullProgressMonitor());
                    updated.add(p.getName());
                } catch (Exception e) {
                    JsonObject f = new JsonObject();
                    f.addProperty("project", p.getName());
                    f.addProperty("error", e.getMessage() == null ? e.getClass().getSimpleName() : e.getMessage());
                    failed.add(f);
                }
            }
            JsonObject result = new JsonObject();
            result.add("updated", updated);
            if (!failed.isEmpty()) result.add("failed", failed);
            result.addProperty("elapsedMs", System.currentTimeMillis() - started);
            return McpToolResult.success(result);
        } catch (ToolSupport.Refusal e) {
            return McpToolResult.error(e.getMessage());
        } catch (Exception | LinkageError e) {
            ClaudeCodeView.debug("[mavenUpdate] failed: " + e);
            return McpToolResult.error("mavenUpdate failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }
}
