package com.anthropic.claudecode.eclipse.tools;

import java.io.File;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IMarker;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.resources.IStorage;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.IPath;
import org.eclipse.debug.core.DebugEvent;
import org.eclipse.debug.core.DebugPlugin;
import org.eclipse.debug.core.IBreakpointListener;
import org.eclipse.debug.core.IBreakpointManager;
import org.eclipse.debug.core.IDebugEventSetListener;
import org.eclipse.debug.core.ILaunch;
import org.eclipse.debug.core.model.IBreakpoint;
import org.eclipse.debug.core.model.IDebugTarget;
import org.eclipse.debug.core.model.IIndexedValue;
import org.eclipse.debug.core.model.ILineBreakpoint;
import org.eclipse.debug.core.model.ISourceLocator;
import org.eclipse.debug.core.model.IStackFrame;
import org.eclipse.debug.core.model.IThread;
import org.eclipse.debug.core.model.IValue;
import org.eclipse.debug.core.model.IVariable;
import org.eclipse.debug.core.model.IWatchExpression;
import org.eclipse.debug.ui.DebugUITools;
import org.eclipse.debug.ui.actions.IToggleBreakpointsTarget;
import org.eclipse.jface.text.IDocument;
import org.eclipse.jface.text.TextSelection;
import org.eclipse.ui.IEditorPart;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.ide.IDE;
import org.eclipse.ui.texteditor.ITextEditor;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Drives Eclipse's debugger: what a paused program is doing (threads, stack, variables),
 * evaluating an expression in a stack frame, line breakpoints, and stepping.
 *
 * <p>Everything goes through the platform debug model ({@code org.eclipse.debug.core}):
 * {@link IDebugTarget} → {@link IThread} → {@link IStackFrame} → {@link IVariable}. Java
 * (JDT), C/C++ (CDT) and Python (PyDev) each implement that model, so one tool debugs all
 * of them with no dependency on any. Language-specific pieces arrive through the model's
 * own extension points and are checked for at call time instead of guarded at load time:
 * <ul>
 * <li>Evaluation needs a watch-expression delegate for the frame's debug model
 *     ({@code IExpressionManager.hasWatchExpressionDelegate}). A private
 *     {@link IWatchExpression} is used — never added to the expression manager, so nothing
 *     appears in the user's Expressions view. It evaluates exactly once, from
 *     {@code setExpressionContext}; calling {@code evaluate()} as well would run an
 *     expression with side effects twice.</li>
 * <li>Setting a line breakpoint goes through the editor's toggle-breakpoints target, the
 *     same path as double-clicking the ruler, so each language validates the line its own
 *     way. JDT does this in a background job and may move the breakpoint to the nearest
 *     line holding code, so the tool waits for the breakpoint to appear and reports the
 *     line it actually landed on.</li>
 * <li>Waiting for a program to stop registers a {@link DebugEvent#SUSPEND} listener before
 *     resuming or stepping. Polling {@link IThread#isSuspended()} straight after the call
 *     can still see the old state.</li>
 * </ul>
 *
 * <p>Uses only {@code org.eclipse.debug.{core,ui}}, {@code org.eclipse.ui.ide},
 * {@code org.eclipse.ui.workbench.texteditor}, {@code org.eclipse.jface.text} and
 * {@code org.eclipse.core.resources}, all hard {@code Require-Bundle}s — no availability guard.
 */
public class DebugTool implements McpTool {

    private static final int DEFAULT_MAX_FRAMES = 10;
    private static final int MAX_MAX_FRAMES = 100;
    private static final int DEFAULT_DEPTH = 1;
    private static final int MAX_DEPTH = 3;
    private static final int MAX_CHILDREN = 50;
    private static final int MAX_VALUE_CHARS = 300;
    private static final long EVALUATE_TIMEOUT_MS = 15_000;
    private static final long BREAKPOINT_TIMEOUT_MS = 5_000;
    /** How long to let a new breakpoint be moved to a line with code (JDT does this). */
    private static final long BREAKPOINT_SETTLE_MS = 1_000;
    private static final int DEFAULT_STEP_WAIT_SECONDS = 5;
    private static final int MAX_WAIT_SECONDS = 300;

    @Override
    public String toolName() {
        return "debug";
    }

    @Override
    public String description() {
        return "Use Eclipse's debugger on a program started in debug mode (runAs with mode='debug', "
                + "or Debug As). Works for any language whose Eclipse tooling provides a debugger. "
                + "Actions: 'state' (default) shows threads, the stack of a paused thread and the "
                + "variables of one frame; 'evaluate' runs an expression in a paused frame; "
                + "'breakpoints' lists line breakpoints; 'setBreakpoint' / 'removeBreakpoint' take "
                + "'file' and 'line'; 'resume', 'stepOver', 'stepInto', 'stepReturn' and 'suspend' "
                + "control execution, and with 'waitSeconds' they wait for the program to stop "
                + "again (at a breakpoint or the end of the step) and return its state. "
                + "'config' picks the launch by configuration name, as 'launches' and 'runAs' "
                + "report it; by default the most recent debug launch is used.";
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");

        JsonObject props = new JsonObject();

        JsonObject action = new JsonObject();
        action.addProperty("type", "string");
        JsonArray actions = new JsonArray();
        for (String a : new String[] { "state", "evaluate", "breakpoints", "setBreakpoint",
                "removeBreakpoint", "resume", "stepOver", "stepInto", "stepReturn", "suspend" }) {
            actions.add(a);
        }
        action.add("enum", actions);
        action.addProperty("description", "What to do (default 'state').");
        props.add("action", action);

        props.add("config", prop("string", "Launch configuration name (case-insensitive). "
                + "Default: the most recent running debug launch, preferring one that is paused."));
        props.add("thread", prop("string", "Thread name. Default: the first paused thread, "
                + "preferring one stopped at a breakpoint."));
        props.add("frame", prop("integer", "Stack frame index within the thread, 0 = top "
                + "(default 0). Used by 'state' for variables and by 'evaluate'."));
        props.add("expression", prop("string", "For 'evaluate': the expression, in the "
                + "program's language."));
        props.add("file", prop("string", "For 'setBreakpoint' / 'removeBreakpoint': absolute "
                + "path of a file inside a workspace project."));
        props.add("line", prop("integer", "For 'setBreakpoint' / 'removeBreakpoint': 1-based line."));
        props.add("depth", prop("integer", "For 'state': how many levels of variable children "
                + "to expand (default " + DEFAULT_DEPTH + ", at most " + MAX_DEPTH + ")."));
        props.add("maxFrames", prop("integer", "For 'state': stack frames to list per paused "
                + "thread (default " + DEFAULT_MAX_FRAMES + ")."));
        props.add("waitSeconds", prop("integer", "For resume/step/suspend: seconds to wait for "
                + "the program to stop before returning (default " + DEFAULT_STEP_WAIT_SECONDS
                + " for steps and suspend, 0 for resume)."));

        schema.add("properties", props);
        return schema;   // no args means "state"
    }

    private static JsonObject prop(String type, String description) {
        JsonObject p = new JsonObject();
        p.addProperty("type", type);
        p.addProperty("description", description);
        return p;
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        String action = EditorUtils.getString(params, "action", "action");
        if (action == null || action.isBlank()) action = "state";
        try {
            ClaudeCodeView.debug("[debug] action=" + action + " params=" + params);
            switch (action) {
                case "state":
                    return state(params);
                case "evaluate":
                    return evaluate(params);
                case "breakpoints":
                    return McpToolResult.success(listBreakpoints());
                case "setBreakpoint":
                    return setBreakpoint(params);
                case "removeBreakpoint":
                    return removeBreakpoint(params);
                case "resume":
                case "stepOver":
                case "stepInto":
                case "stepReturn":
                case "suspend":
                    return control(action, params);
                default:
                    return McpToolResult.error("Unknown action: " + action);
            }
        } catch (Exception e) {
            ClaudeCodeView.debug("[debug] " + action + " failed: " + e);
            return McpToolResult.error("debug " + action + " failed: " + e.getClass().getSimpleName()
                    + ": " + e.getMessage());
        }
    }

    // ── Launch / thread / frame selection ───────────────────────────────────────────

    /**
     * The debug launch named {@code config}, or the best one when null: running beats
     * terminated, then paused beats not paused, then newer beats older.
     */
    private static ILaunch pickLaunch(String config) {
        ILaunch best = null;
        int bestScore = -1;
        ILaunch[] launches = DebugPlugin.getDefault().getLaunchManager().getLaunches();
        for (int i = 0; i < launches.length; i++) {
            ILaunch l = launches[i];
            if (l.getDebugTargets().length == 0) continue;
            if (config != null && !config.equalsIgnoreCase(LaunchesTool.nameOf(l))) continue;
            int score = (l.isTerminated() ? 0 : 2) + (hasSuspendedThread(l) ? 1 : 0);
            if (score >= bestScore) {   // >= so a newer launch wins a tie
                best = l;
                bestScore = score;
            }
        }
        return best;
    }

    private static boolean hasSuspendedThread(ILaunch launch) {
        for (IDebugTarget t : launch.getDebugTargets()) {
            try {
                for (IThread th : t.getThreads()) if (th.isSuspended()) return true;
            } catch (Exception ignored) {
                // A target that is going away — count it as not paused.
            }
        }
        return false;
    }

    private static McpToolResult noLaunch(String config) {
        return McpToolResult.error(config == null
                ? "No debug launch. Start one with runAs mode='debug' (or Debug As), then try again."
                : "No debug launch named '" + config + "'. Call 'launches' to see what is running; "
                        + "the program must have been started in debug mode.");
    }

    /** The named thread, or the first paused one — preferring one stopped at a breakpoint. */
    private static IThread pickThread(ILaunch launch, String name) throws Exception {
        IThread firstSuspended = null;
        IThread firstAny = null;
        for (IDebugTarget t : launch.getDebugTargets()) {
            if (t.isTerminated()) continue;
            for (IThread th : t.getThreads()) {
                if (name != null) {
                    if (name.equals(th.getName())) return th;
                    continue;
                }
                if (firstAny == null) firstAny = th;
                if (th.isSuspended()) {
                    if (th.getBreakpoints().length > 0) return th;
                    if (firstSuspended == null) firstSuspended = th;
                }
            }
        }
        return name != null ? null : firstSuspended != null ? firstSuspended : firstAny;
    }

    private static IStackFrame pickFrame(IThread thread, int index) throws Exception {
        if (!thread.isSuspended()) return null;
        IStackFrame[] frames = thread.getStackFrames();
        return index >= 0 && index < frames.length ? frames[index] : null;
    }

    // ── state ───────────────────────────────────────────────────────────────────────

    private McpToolResult state(JsonObject params) throws Exception {
        String config = EditorUtils.getString(params, "config", "config");
        ILaunch launch = pickLaunch(config);
        if (launch == null) return noLaunch(config);
        return McpToolResult.success(describe(launch,
                EditorUtils.getString(params, "thread", "thread"),
                EditorUtils.getInt(params, "frame", "frame", 0),
                clamp(EditorUtils.getInt(params, "depth", "depth", DEFAULT_DEPTH), 0, MAX_DEPTH),
                clamp(EditorUtils.getInt(params, "maxFrames", "maxFrames", DEFAULT_MAX_FRAMES), 1, MAX_MAX_FRAMES)));
    }

    /**
     * Threads of every target in the launch; the stack of each paused thread; and the
     * variables of one frame of one thread — the selected one.
     */
    private static JsonObject describe(ILaunch launch, String threadName, int frameIndex, int depth,
            int maxFrames) throws Exception {
        JsonObject out = new JsonObject();
        out.addProperty("config", LaunchesTool.nameOf(launch));
        out.addProperty("mode", launch.getLaunchMode());
        out.addProperty("terminated", launch.isTerminated());

        IThread selected = launch.isTerminated() ? null : pickThread(launch, threadName);
        ISourceLocator locator = launch.getSourceLocator();

        JsonArray threads = new JsonArray();
        for (IDebugTarget target : launch.getDebugTargets()) {
            if (target.isTerminated()) continue;
            for (IThread th : target.getThreads()) {
                JsonObject tj = new JsonObject();
                tj.addProperty("name", safeName(th));
                tj.addProperty("state", th.isSuspended() ? "suspended"
                        : th.isStepping() ? "stepping" : "running");
                if (th.isSuspended()) {
                    IBreakpoint[] hit = th.getBreakpoints();
                    if (hit.length > 0) tj.addProperty("stoppedAt", describeBreakpoint(hit[0]));
                    JsonArray frames = new JsonArray();
                    IStackFrame[] stack = th.getStackFrames();
                    for (int i = 0; i < stack.length && i < maxFrames; i++) {
                        frames.add(frameJson(stack[i], i, locator));
                    }
                    if (stack.length > maxFrames) {
                        tj.addProperty("moreFrames", stack.length - maxFrames);
                    }
                    tj.add("frames", frames);
                }
                threads.add(tj);
            }
        }
        out.add("threads", threads);
        for (IDebugTarget target : launch.getDebugTargets()) {
            // Filled only under JDT (see HotCodeReplaceStatus); absent otherwise.
            String hcr = HotCodeReplaceStatus.get(target);
            if (hcr != null) out.addProperty("hotCodeReplace", hcr);
        }

        if (selected != null && selected.isSuspended()) {
            IStackFrame frame = pickFrame(selected, frameIndex);
            if (frame != null) {
                JsonObject vj = new JsonObject();
                vj.addProperty("thread", safeName(selected));
                vj.addProperty("frame", frameIndex);
                vj.addProperty("frameName", frame.getName());
                vj.add("variables", variablesJson(frame.getVariables(), depth));
                out.add("selectedFrame", vj);
            } else {
                out.addProperty("note", "Thread '" + safeName(selected) + "' has no frame " + frameIndex + ".");
            }
        } else if (!launch.isTerminated()) {
            out.addProperty("note", threadName != null && selected == null
                    ? "No thread named '" + threadName + "'."
                    : "No thread is paused. Set a breakpoint, or use action='suspend'.");
        }
        return out;
    }

    private static JsonObject frameJson(IStackFrame frame, int index, ISourceLocator locator) {
        JsonObject fj = new JsonObject();
        fj.addProperty("index", index);
        try { fj.addProperty("name", frame.getName()); } catch (Exception ignored) {}
        try {
            int line = frame.getLineNumber();
            if (line > 0) fj.addProperty("line", line);
        } catch (Exception ignored) {}
        String source = sourceOf(frame, locator);
        if (source != null) fj.addProperty("source", source);
        return fj;
    }

    /** Where the debugger's source lookup finds the frame's code, if anywhere. */
    private static String sourceOf(IStackFrame frame, ISourceLocator locator) {
        if (locator == null) return null;
        try {
            Object element = locator.getSourceElement(frame);
            if (element instanceof IFile file) {
                IPath location = file.getLocation();
                return location != null ? location.toOSString() : file.getFullPath().toString();
            }
            if (element instanceof IStorage storage) {
                IPath path = storage.getFullPath();
                return path != null ? path.toString() : storage.getName();
            }
            if (element instanceof File file) return file.getAbsolutePath();
        } catch (Exception ignored) {
            // Source lookup is best-effort; the frame name still says where it is.
        }
        return null;
    }

    private static JsonArray variablesJson(IVariable[] variables, int depth) {
        JsonArray arr = new JsonArray();
        for (int i = 0; i < variables.length && i < MAX_CHILDREN; i++) {
            arr.add(variableJson(variables[i], depth));
        }
        if (variables.length > MAX_CHILDREN) {
            JsonObject more = new JsonObject();
            more.addProperty("more", variables.length - MAX_CHILDREN);
            arr.add(more);
        }
        return arr;
    }

    private static JsonObject variableJson(IVariable variable, int depth) {
        JsonObject vj = new JsonObject();
        try {
            vj.addProperty("name", variable.getName());
            try { vj.addProperty("type", variable.getReferenceTypeName()); } catch (Exception ignored) {}
            IValue value = variable.getValue();
            vj.addProperty("value", clip(value.getValueString()));
            // A Java string's value already is its text; its fields (hash, coder, the byte
            // array) are noise. The type name only ever matches under JDT.
            boolean opaque = "java.lang.String".equals(value.getReferenceTypeName());
            if (depth > 0 && !opaque && value.hasVariables()) {
                if (value instanceof IIndexedValue indexed) {
                    // Only fetch the slice shown — a large array's elements are fetched one by one.
                    int size = indexed.getSize();
                    int n = Math.min(size, MAX_CHILDREN);
                    JsonArray children = variablesJson(indexed.getVariables(indexed.getInitialOffset(), n), depth - 1);
                    if (size > n) {
                        JsonObject more = new JsonObject();
                        more.addProperty("more", size - n);
                        children.add(more);
                    }
                    vj.add("children", children);
                } else {
                    vj.add("children", variablesJson(value.getVariables(), depth - 1));
                }
            }
        } catch (Exception e) {
            vj.addProperty("error", e.getMessage());
        }
        return vj;
    }

    // ── evaluate ────────────────────────────────────────────────────────────────────

    private McpToolResult evaluate(JsonObject params) throws Exception {
        String expression = EditorUtils.getString(params, "expression", "expression");
        if (expression == null || expression.isBlank()) {
            return McpToolResult.error("action='evaluate' requires 'expression'.");
        }
        String config = EditorUtils.getString(params, "config", "config");
        ILaunch launch = pickLaunch(config);
        if (launch == null) return noLaunch(config);
        String threadName = EditorUtils.getString(params, "thread", "thread");
        IThread thread = pickThread(launch, threadName);
        if (thread == null || !thread.isSuspended()) {
            return McpToolResult.error("Evaluation needs a paused thread"
                    + (threadName != null ? " named '" + threadName + "'" : "")
                    + ". Set a breakpoint and wait for it, or use action='suspend'.");
        }
        int frameIndex = EditorUtils.getInt(params, "frame", "frame", 0);
        IStackFrame frame = pickFrame(thread, frameIndex);
        if (frame == null) return McpToolResult.error("Thread has no frame " + frameIndex + ".");

        if (!DebugPlugin.getDefault().getExpressionManager()
                .hasWatchExpressionDelegate(frame.getModelIdentifier())) {
            return McpToolResult.error("This debugger (" + frame.getModelIdentifier()
                    + ") does not support evaluating expressions.");
        }

        // Evaluates once, from setExpressionContext — see the class comment.
        IWatchExpression watch = DebugPlugin.getDefault().getExpressionManager()
                .newWatchExpression(expression);
        try {
            watch.setExpressionContext(frame);
            long deadline = System.currentTimeMillis() + EVALUATE_TIMEOUT_MS;
            while (watch.isPending() && System.currentTimeMillis() < deadline) {
                Thread.sleep(50);
            }
            JsonObject out = new JsonObject();
            out.addProperty("expression", expression);
            out.addProperty("thread", safeName(thread));
            out.addProperty("frame", frameIndex);
            if (watch.isPending()) {
                out.addProperty("error", "No result within " + (EVALUATE_TIMEOUT_MS / 1000)
                        + "s; the evaluation may still be running in the program.");
                return McpToolResult.success(out);
            }
            if (watch.hasErrors()) {
                JsonArray errors = new JsonArray();
                for (String m : watch.getErrorMessages()) errors.add(m);
                out.add("errors", errors);
                return McpToolResult.success(out);
            }
            IValue value = watch.getValue();
            if (value == null) {
                out.addProperty("error", "The evaluation produced no value.");
                return McpToolResult.success(out);
            }
            out.addProperty("value", clip(value.getValueString()));
            try { out.addProperty("type", value.getReferenceTypeName()); } catch (Exception ignored) {}
            if (value.hasVariables()) out.add("children", variablesJson(value.getVariables(), 0));
            return McpToolResult.success(out);
        } finally {
            watch.dispose();
        }
    }

    // ── breakpoints ─────────────────────────────────────────────────────────────────

    private static JsonObject listBreakpoints() {
        IBreakpointManager manager = DebugPlugin.getDefault().getBreakpointManager();
        JsonArray arr = new JsonArray();
        int others = 0;
        for (IBreakpoint bp : manager.getBreakpoints()) {
            if (!(bp instanceof ILineBreakpoint)) {
                others++;
                continue;
            }
            JsonObject bj = new JsonObject();
            bj.addProperty("breakpoint", describeBreakpoint(bp));
            try { bj.addProperty("enabled", bp.isEnabled()); } catch (Exception ignored) {}
            String type = manager.getTypeName(bp);
            if (type != null) bj.addProperty("type", type);
            arr.add(bj);
        }
        JsonObject out = new JsonObject();
        out.addProperty("skipAll", !manager.isEnabled());
        out.add("lineBreakpoints", arr);
        if (others > 0) out.addProperty("otherBreakpoints", others);
        return out;
    }

    /** {@code path:line}, or the marker's resource when there is no line. */
    private static String describeBreakpoint(IBreakpoint bp) {
        IMarker marker = bp.getMarker();
        if (marker == null) return bp.getModelIdentifier();
        IResource resource = marker.getResource();
        IPath location = resource != null ? resource.getLocation() : null;
        String where = location != null ? location.toOSString()
                : resource != null ? resource.getFullPath().toString() : "?";
        if (bp instanceof ILineBreakpoint line) {
            try { return where + ":" + line.getLineNumber(); } catch (Exception ignored) {}
        }
        return where;
    }

    private static ILineBreakpoint findLineBreakpoint(IFile file, int line) {
        for (IBreakpoint bp : DebugPlugin.getDefault().getBreakpointManager().getBreakpoints()) {
            if (!(bp instanceof ILineBreakpoint lb) || bp.getMarker() == null) continue;
            if (!file.equals(bp.getMarker().getResource())) continue;
            try {
                if (lb.getLineNumber() == line) return lb;
            } catch (Exception ignored) {}
        }
        return null;
    }

    private static IFile workspaceFile(String path) {
        IFile[] files = ResourcesPlugin.getWorkspace().getRoot()
                .findFilesForLocationURI(new File(path).toURI());
        for (IFile f : files) if (f.exists()) return f;
        return null;
    }

    private McpToolResult setBreakpoint(JsonObject params) throws Exception {
        String path = EditorUtils.getString(params, "file", "file");
        int line = EditorUtils.getInt(params, "line", "line", 0);
        if (path == null || line < 1) {
            return McpToolResult.error("action='setBreakpoint' requires 'file' and a 1-based 'line'.");
        }
        IFile file = workspaceFile(path);
        if (file == null) {
            return McpToolResult.error("Not a file in a workspace project: " + path
                    + ". Breakpoints are set through the file's editor, so it must be in the workspace.");
        }
        ILineBreakpoint existing = findLineBreakpoint(file, line);
        if (existing != null) {
            JsonObject out = new JsonObject();
            out.addProperty("breakpoint", describeBreakpoint(existing));
            out.addProperty("created", false);
            out.addProperty("note", "A breakpoint is already set on this line.");
            return McpToolResult.success(out);
        }

        // Listen before toggling: JDT creates the breakpoint later, from a job. On a line with
        // no code it first creates one there, then deletes it and creates another on the
        // nearest line that has code — so collect every breakpoint added to the file, let the
        // job settle, and report the newest one that still exists.
        List<IBreakpoint> added = new CopyOnWriteArrayList<>();
        CountDownLatch addedLatch = new CountDownLatch(1);
        IBreakpointListener listener = new IBreakpointListener() {
            @Override
            public void breakpointAdded(IBreakpoint bp) {
                if (bp.getMarker() != null && file.equals(bp.getMarker().getResource())) {
                    added.add(bp);
                    addedLatch.countDown();
                }
            }
            @Override public void breakpointChanged(IBreakpoint bp, org.eclipse.core.resources.IMarkerDelta delta) { }
            @Override public void breakpointRemoved(IBreakpoint bp, org.eclipse.core.resources.IMarkerDelta delta) { }
        };
        IBreakpointManager manager = DebugPlugin.getDefault().getBreakpointManager();
        manager.addBreakpointListener(listener);
        try {
            String problem = UiHelper.syncCall(() -> toggleLine(file, line));
            if (problem != null) return McpToolResult.error(problem);
            if (addedLatch.await(BREAKPOINT_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
                Thread.sleep(BREAKPOINT_SETTLE_MS);
            }
        } finally {
            manager.removeBreakpointListener(listener);
        }

        IBreakpoint bp = null;
        for (int i = added.size() - 1; i >= 0 && bp == null; i--) {
            IBreakpoint candidate = added.get(i);
            if (candidate.getMarker() != null && candidate.getMarker().exists()) bp = candidate;
        }
        if (bp == null) {
            return McpToolResult.error("No breakpoint was created on line " + line + " of " + path
                    + " within " + (BREAKPOINT_TIMEOUT_MS / 1000) + "s. The line may hold no "
                    + "executable code.");
        }
        JsonObject out = new JsonObject();
        out.addProperty("breakpoint", describeBreakpoint(bp));
        out.addProperty("created", true);
        if (bp instanceof ILineBreakpoint lb && lb.getLineNumber() != line) {
            out.addProperty("note", "Placed on line " + lb.getLineNumber()
                    + ", the nearest line the debugger accepts.");
        }
        ClaudeCodeView.debug("[debug] breakpoint set: " + describeBreakpoint(bp));
        return McpToolResult.success(out);
    }

    /** On the UI thread: toggles a line breakpoint via the file's editor. Null on success. */
    private static String toggleLine(IFile file, int line) {
        try {
            IWorkbenchPage page = UiHelper.getActivePage();
            if (page == null) return "No workbench window is open.";
            IEditorPart editor = IDE.openEditor(page, file, false);
            ITextEditor textEditor = editor == null ? null : editor.getAdapter(ITextEditor.class);
            if (textEditor == null) return "The file did not open in a text editor.";
            IDocument doc = textEditor.getDocumentProvider().getDocument(textEditor.getEditorInput());
            if (line > doc.getNumberOfLines()) {
                return "Line " + line + " is past the end of the file (" + doc.getNumberOfLines() + " lines).";
            }
            TextSelection selection = new TextSelection(doc, doc.getLineOffset(line - 1), 0);
            IToggleBreakpointsTarget target = DebugUITools.getToggleBreakpointsTargetManager()
                    .getToggleBreakpointsTarget(editor, selection);
            if (target == null || !target.canToggleLineBreakpoints(editor, selection)) {
                return "Line breakpoints are not supported in this file's editor.";
            }
            target.toggleLineBreakpoints(editor, selection);
            return null;
        } catch (Exception e) {
            return "Could not set the breakpoint: " + e.getClass().getSimpleName() + ": " + e.getMessage();
        }
    }

    private McpToolResult removeBreakpoint(JsonObject params) throws Exception {
        String path = EditorUtils.getString(params, "file", "file");
        int line = EditorUtils.getInt(params, "line", "line", 0);
        if (path == null || line < 1) {
            return McpToolResult.error("action='removeBreakpoint' requires 'file' and a 1-based 'line'.");
        }
        IFile file = workspaceFile(path);
        ILineBreakpoint bp = file == null ? null : findLineBreakpoint(file, line);
        if (bp == null) return McpToolResult.error("No line breakpoint at " + path + ":" + line + ".");
        String described = describeBreakpoint(bp);
        DebugPlugin.getDefault().getBreakpointManager().removeBreakpoint(bp, true);
        JsonObject out = new JsonObject();
        out.addProperty("removed", described);
        ClaudeCodeView.debug("[debug] breakpoint removed: " + described);
        return McpToolResult.success(out);
    }

    // ── resume / step / suspend ─────────────────────────────────────────────────────

    private McpToolResult control(String action, JsonObject params) throws Exception {
        String config = EditorUtils.getString(params, "config", "config");
        ILaunch launch = pickLaunch(config);
        if (launch == null) return noLaunch(config);
        if (launch.isTerminated()) return McpToolResult.error("The launch has terminated.");
        String threadName = EditorUtils.getString(params, "thread", "thread");
        boolean step = action.startsWith("step");
        int waitSeconds = clamp(EditorUtils.getInt(params, "waitSeconds", "waitSeconds",
                "resume".equals(action) ? 0 : DEFAULT_STEP_WAIT_SECONDS), 0, MAX_WAIT_SECONDS);

        IThread thread = threadName != null || step ? pickThread(launch, threadName) : null;
        if ((threadName != null || step) && thread == null) {
            return McpToolResult.error(threadName != null ? "No thread named '" + threadName + "'."
                    : "No paused thread to step.");
        }
        if (step && !thread.isSuspended()) {
            return McpToolResult.error("Thread '" + safeName(thread) + "' is not paused, so it cannot step.");
        }

        // Listen before acting, so a stop that follows immediately is not missed.
        SuspendWaiter waiter = new SuspendWaiter(launch);
        DebugPlugin.getDefault().addDebugEventListener(waiter);
        try {
            switch (action) {
                case "stepOver" -> thread.stepOver();
                case "stepInto" -> thread.stepInto();
                case "stepReturn" -> thread.stepReturn();
                case "resume" -> {
                    if (thread != null) thread.resume();
                    else for (IDebugTarget t : launch.getDebugTargets()) if (t.canResume()) t.resume();
                }
                case "suspend" -> {
                    if (thread != null) thread.suspend();
                    else for (IDebugTarget t : launch.getDebugTargets()) if (t.canSuspend()) t.suspend();
                }
                default -> { }
            }
            boolean signalled = waitSeconds > 0 && waiter.await(waitSeconds);
            boolean ended = signalled && waiter.terminated();
            if (ended) {
                // The target reports TERMINATE before its process does; let the launch catch up
                // so 'terminated' below is accurate.
                long deadline = System.currentTimeMillis() + 2000;
                while (!launch.isTerminated() && System.currentTimeMillis() < deadline) Thread.sleep(50);
            }

            JsonObject out;
            if (signalled && !ended) {
                out = describe(launch, waiter.threadName(threadName), 0, DEFAULT_DEPTH, DEFAULT_MAX_FRAMES);
            } else {
                out = new JsonObject();
                out.addProperty("config", LaunchesTool.nameOf(launch));
                out.addProperty("terminated", launch.isTerminated());
            }
            out.addProperty("action", action);
            if (ended) {
                out.addProperty("note", "The program terminated. Its output: 'launches' action='output'.");
            } else if (waitSeconds > 0 && !signalled) {
                out.addProperty("note", "Still running after " + waitSeconds + "s — no breakpoint was "
                        + "hit. Call action='state' later, or 'suspend'.");
            }
            return McpToolResult.success(out);
        } finally {
            DebugPlugin.getDefault().removeDebugEventListener(waiter);
        }
    }

    /** Waits for any thread of the launch to suspend, or for the launch to terminate. */
    private static final class SuspendWaiter implements IDebugEventSetListener {
        private final ILaunch launch;
        private final CountDownLatch latch = new CountDownLatch(1);
        private volatile String suspendedThread;
        private volatile boolean terminated;

        SuspendWaiter(ILaunch launch) {
            this.launch = launch;
        }

        @Override
        public void handleDebugEvents(DebugEvent[] events) {
            for (DebugEvent event : events) {
                Object source = event.getSource();
                if (event.getKind() == DebugEvent.SUSPEND && source instanceof IThread th
                        && th.getLaunch() == launch) {
                    // An evaluation suspends and resumes the thread behind the scenes.
                    if (event.getDetail() == DebugEvent.EVALUATION_IMPLICIT) continue;
                    suspendedThread = safeName(th);
                    latch.countDown();
                } else if (event.getKind() == DebugEvent.SUSPEND && source instanceof IDebugTarget t
                        && t.getLaunch() == launch) {
                    latch.countDown();
                } else if (event.getKind() == DebugEvent.TERMINATE && source instanceof IDebugTarget t
                        && t.getLaunch() == launch) {
                    terminated = true;
                    latch.countDown();
                }
            }
        }

        /** True once the program stopped or ended; {@link #terminated()} says which. */
        boolean await(int seconds) throws InterruptedException {
            return latch.await(seconds, TimeUnit.SECONDS);
        }

        boolean terminated() {
            return terminated;
        }

        /** The thread that stopped, so the state shown is the one that just paused. */
        String threadName(String requested) {
            return requested != null ? requested : suspendedThread;
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────────────────

    private static String safeName(IThread thread) {
        try {
            return thread.getName();
        } catch (Exception e) {
            return "(unknown thread)";
        }
    }

    private static String clip(String s) {
        if (s == null) return null;
        return s.length() <= MAX_VALUE_CHARS ? s : s.substring(0, MAX_VALUE_CHARS) + "…";
    }

    private static int clamp(int v, int min, int max) {
        return Math.max(min, Math.min(max, v));
    }
}
