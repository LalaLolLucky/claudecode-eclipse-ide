package com.anthropic.claudecode.eclipse.tools;

import java.util.ArrayList;
import java.util.List;
import java.util.regex.Pattern;
import java.util.regex.PatternSyntaxException;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IProject;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.core.runtime.OperationCanceledException;
import org.eclipse.search.core.text.TextSearchEngine;
import org.eclipse.search.core.text.TextSearchMatchAccess;
import org.eclipse.search.core.text.TextSearchRequestor;
import org.eclipse.search.core.text.TextSearchScope;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Searches file contents with Eclipse's own File Search engine (Search &gt; File…).
 *
 * <p>It searches the workspace as Eclipse holds it, which is not the same as the files on
 * disk: a file open in an editor is searched as it stands there, unsaved changes included;
 * a folder linked into a project from elsewhere is searched as part of it; and build output
 * (derived resources) is left out. That is what it is for — a search of the files on disk
 * is what Claude's own search tool does, and does without Eclipse.
 *
 * <p>The engine is in {@code org.eclipse.search}, an optional {@code Require-Bundle}; the
 * registry leaves this tool out where it is not installed.
 */
public class WorkspaceSearchTool implements McpTool {

    private static final int LISTED = 100;
    private static final int MOST_LISTED = 1000;

    /** How much of a matching line is shown. */
    private static final int MAX_LINE = 300;

    @Override
    public String toolName() {
        return "workspaceSearch";
    }

    @Override
    public String description() {
        return "Search file contents with Eclipse's File Search, over the workspace as Eclipse holds "
                + "it: files open in an editor are searched with their unsaved changes, folders "
                + "linked into a project are included, and build output is skipped unless "
                + "'includeDerived' is true. Use it when one of those matters; for an ordinary "
                + "search of the files on disk, the usual search tool is the one to use. 'pattern' "
                + "is plain text, or a Java regular expression with 'regex' true. Scope: the named "
                + "projects, or the whole workspace when none are given, narrowed by 'files'.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject props = new JsonObject();
        props.add("pattern", ToolSupport.prop("string", "The text to find."));
        props.add("regex", ToolSupport.prop("boolean", "true reads 'pattern' as a Java regular expression (default false)."));
        props.add("caseSensitive", ToolSupport.prop("boolean", "Match case (default false)."));
        props.add("files", ToolSupport.prop("string", "File name patterns, comma-separated, with * and ?, "
                + "e.g. '*.java, *.xml'. Left out, every file."));
        props.add("projects", ToolSupport.strings("Project names to search. Omit or leave empty for the whole workspace."));
        props.add("includeDerived", ToolSupport.prop("boolean", "Also search build output (default false)."));
        props.add("limit", ToolSupport.prop("integer",
                "How many matches to list, 1 to " + MOST_LISTED + "; " + LISTED + " when left out."));
        return ToolSupport.schema(props, "pattern");
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        ClaudeCodeView.debug("[workspaceSearch] " + params);
        try {
            String text = params.has("pattern") && params.get("pattern").isJsonPrimitive()
                    ? params.get("pattern").getAsString() : "";
            if (text.isEmpty()) return McpToolResult.error("Missing required parameter: pattern");
            int flags = ToolSupport.bool(params, "caseSensitive", false) ? 0
                    : Pattern.CASE_INSENSITIVE | Pattern.UNICODE_CASE;
            Pattern pattern;
            try {
                pattern = Pattern.compile(ToolSupport.bool(params, "regex", false) ? text : Pattern.quote(text),
                        flags | Pattern.MULTILINE);
            } catch (PatternSyntaxException e) {
                return McpToolResult.error("'pattern' is not a valid regular expression: " + e.getDescription());
            }
            int limit = ToolSupport.integer(params, "limit", LISTED, 1, MOST_LISTED);
            List<IProject> projects = ToolSupport.projects(params, "search");

            TextSearchScope scope = TextSearchScope.newSearchScope(projects.toArray(new IResource[0]),
                    fileNames(ToolSupport.str(params, "files")), ToolSupport.bool(params, "includeDerived", false));

            JsonArray matches = new JsonArray();
            List<String> files = new ArrayList<>();
            NullProgressMonitor monitor = new NullProgressMonitor();
            TextSearchRequestor requestor = new TextSearchRequestor() {
                // Where the line count of the file being read has got to. Matches of a file
                // arrive in order, so each one counts on from the one before.
                private int countedTo;
                private int line;

                @Override
                public boolean canRunInParallel() {
                    return false;   // the fields above are one file's at a time
                }

                @Override
                public boolean acceptFile(IFile file) {
                    countedTo = 0;
                    line = 1;
                    return !monitor.isCanceled();
                }

                @Override
                public boolean acceptPatternMatch(TextSearchMatchAccess match) {
                    if (matches.size() >= limit) {
                        monitor.setCanceled(true);   // one more than asked for: there is more
                        return false;
                    }
                    int offset = match.getMatchOffset();
                    for (; countedTo < offset; countedTo++) {
                        if (match.getFileContentChar(countedTo) == '\n') line++;
                    }
                    int start = offset;
                    while (start > 0 && offset - start < MAX_LINE && !isLineEnd(match.getFileContentChar(start - 1))) start--;
                    int end = offset;
                    int length = match.getFileContentLength();
                    while (end < length && end - start < MAX_LINE && !isLineEnd(match.getFileContentChar(end))) end++;

                    IFile file = match.getFile();
                    String path = file.getLocation() != null ? file.getLocation().toOSString() : file.getFullPath().toString();
                    if (files.isEmpty() || !files.get(files.size() - 1).equals(path)) files.add(path);
                    JsonObject m = new JsonObject();
                    m.addProperty("file", path);
                    m.addProperty("line", line);
                    m.addProperty("text", match.getFileContent(start, end - start).strip());
                    matches.add(m);
                    return true;
                }
            };
            try {
                TextSearchEngine.create().search(scope, requestor, pattern, monitor);
            } catch (OperationCanceledException e) {
                // stopped at the limit, by the requestor above
            }

            JsonObject result = new JsonObject();
            result.addProperty("matchCount", matches.size());
            result.addProperty("fileCount", files.size());
            result.add("matches", matches);
            if (monitor.isCanceled()) {
                result.addProperty("truncated", true);
                result.addProperty("note", "There are more matches than the " + limit + " listed.");
            }
            return McpToolResult.success(result);
        } catch (ToolSupport.Refusal e) {
            return McpToolResult.error(e.getMessage());
        } catch (Exception | LinkageError e) {
            ClaudeCodeView.debug("[workspaceSearch] failed: " + e);
            return McpToolResult.error("workspaceSearch failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }

    private static boolean isLineEnd(char c) {
        return c == '\n' || c == '\r';
    }

    /** 'files' as the pattern a file's name has to match: "*.java, *.xml" → {@code .*\.java|.*\.xml}. */
    static Pattern fileNames(String files) {
        List<String> alternatives = new ArrayList<>();
        if (files != null) {
            for (String glob : files.split(",")) {
                glob = glob.trim();
                if (glob.isEmpty()) continue;
                StringBuilder regex = new StringBuilder();
                StringBuilder literal = new StringBuilder();
                for (char c : glob.toCharArray()) {
                    if (c != '*' && c != '?') {
                        literal.append(c);
                        continue;
                    }
                    if (literal.length() > 0) regex.append(Pattern.quote(literal.toString()));
                    literal.setLength(0);
                    regex.append(c == '*' ? ".*" : ".");
                }
                if (literal.length() > 0) regex.append(Pattern.quote(literal.toString()));
                alternatives.add(regex.toString());
            }
        }
        return Pattern.compile(alternatives.isEmpty() ? ".*" : String.join("|", alternatives),
                Pattern.CASE_INSENSITIVE);
    }
}
