package com.anthropic.claudecode.eclipse.tools;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.concurrent.atomic.AtomicReference;

import org.eclipse.swt.widgets.Display;
import org.eclipse.swt.widgets.Shell;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Runs a tool call and notices when it gets stuck behind a dialog it raised.
 *
 * <p>A tool that opens a modal dialog from its UI code (a launcher asking which type to run,
 * "errors exist, proceed?") does not return until the dialog is answered, and the client
 * waits on that one call, so it could never send the call that answers it. Here the call is
 * answered early with the dialog's contents instead, and keeps running: once
 * {@code eclipseDialog} has pressed a button, the original result is handed back with that
 * call.
 */
public final class ToolDialogWatch {

    /** Tools that wait on the user by design, and the tool that answers dialogs. */
    private static final Set<String> EXEMPT =
            Set.of("approvalPrompt", "askUserQuestion", "openDiff", "eclipseDialog");

    private static final long POLL_MS = 400;
    /** No dialog check before this: nearly every call is back by then. */
    private static final long GRACE_MS = 1200;
    private static final long UI_ANSWER_MS = 300;
    /**
     * A dialog that outlasts this while the call is still out is reported when a thread is
     * held behind the UI thread, whichever one that is. A dialog the user opened has none.
     */
    private static final long REPORT_ANYWAY_MS = 5000;
    /** After such a dialog was found to hold no thread, how long before asking again. */
    private static final long HELD_RECHECK_MS = 2000;
    /** How long the UI thread may stay silent under a waiting call before native dialogs are asked for. */
    private static final long SILENT_UI_MS = 3000;
    private static final int MAX_PARKED = 8;

    private static final ExecutorService CALLS = Executors.newCachedThreadPool(work -> {
        Thread thread = new Thread(work, "claude-tool-call");
        thread.setDaemon(true);
        return thread;
    });

    /**
     * A call left running behind a dialog: the token its caller was given, and the dialogs it
     * was reported with. Every conversation reaches this one list — a tool call arrives with
     * no word of who sent it — so a result is handed out only against one of those two.
     */
    private record Parked(String token, String tool, Future<McpToolResult> call, Set<String> dialogs) {
    }

    private static final List<Parked> PARKED = new ArrayList<>();

    /**
     * The dialogs open at one moment: the shells, their titles as {@code [{title}]}, the
     * ones that only report progress, and the ids the native core lists them under where it
     * reports them too ({@link Dialogs#nativeIdsOf}). All of it is read on the UI thread
     * when the look is taken, because the loop that compares two looks runs on the caller's
     * thread, where asking a widget anything is an invalid thread access.
     */
    private record Seen(List<Shell> shells, JsonArray titles, Set<Shell> progress, Set<String> nativeIds) {
    }

    private ToolDialogWatch() {
    }

    public static McpToolResult run(McpTool tool, String toolName, JsonObject args) throws Exception {
        Display display = Dialogs.display();
        if (display == null || EXEMPT.contains(toolName)) return tool.execute(args);

        // What is open before the call, so it is not taken for the call's doing. Queued and
        // not waited for: the UI thread's queue is first in, first out, so this runs ahead of
        // anything the call sends there, and the call does not start a round trip late.
        CompletableFuture<Seen> before = Dialogs.postUi(() -> see(display));

        AtomicReference<Thread> runner = new AtomicReference<>();
        Future<McpToolResult> call = CALLS.submit(() -> {
            runner.set(Thread.currentThread());
            return tool.execute(args);
        });

        long started = System.currentTimeMillis();
        boolean wasWaiting = false;
        Shell lingering = null;
        long lingeringSince = 0;
        Set<Long> heldBefore = null;
        long heldCheckAt = 0;
        while (true) {
            try {
                return call.get(POLL_MS, TimeUnit.MILLISECONDS);
            } catch (TimeoutException stillRunning) {
                // fall through to the dialog check
            } catch (ExecutionException e) {
                if (e.getCause() instanceof Exception cause) throw cause;
                throw e;
            }
            long now = System.currentTimeMillis();
            if (now - started < GRACE_MS) continue;

            Thread thread = runner.get();
            boolean waiting = thread != null && waitingOnUiThread(thread.getStackTrace());
            // Waiting on two checks in a row with the UI thread answering in between: its
            // runnable is not merely queued or slow, it is inside a nested event loop.
            boolean stuck = waiting && wasWaiting;
            wasWaiting = waiting;

            Seen open = look(display);
            Seen was = seenBefore(before);
            JsonArray report = null;
            String why = null;
            if (open == null) {
                // The UI thread is silent. Under a native dialog on a platform whose native
                // loop does not run SWT's queue, that is what a dialog looks like.
                if (stuck && now - started >= SILENT_UI_MS) {
                    report = NativeDialogs.withoutThoseIn(was == null ? new JsonArray() : was.titles(),
                            nativeDialogsHere(), was == null ? Set.of() : was.nativeIds());
                    why = "native dialog, UI thread silent";
                }
            } else {
                List<Shell> raised = raisedBy(open.shells(), was == null ? null : was.shells(), open.progress());
                if (stuck) {
                    report = describe(raised);
                    why = "dialog raised by the call";
                    if (report == null || report.size() == 0) {
                        report = NativeDialogs.withoutThoseIn(open.titles(), nativeDialogsHere(), open.nativeIds());
                        why = "native dialog raised by the call";
                    }
                } else if (raised.isEmpty()) {
                    lingering = null;
                    heldBefore = null;
                } else if (raised.get(0) != lingering) {
                    lingering = raised.get(0);
                    lingeringSince = now;
                    heldBefore = null;
                    heldCheckAt = 0;
                } else if (now - lingeringSince >= REPORT_ANYWAY_MS && now >= heldCheckAt) {
                    // Raised on the call's behalf by another thread: a job it is waiting for.
                    // That thread stays inside syncExec for as long as the dialog is up, so it
                    // is there on two checks in a row. A dialog the user opened holds nobody:
                    // its event loop runs each thread's request and lets it go, and a thread
                    // caught passing through is gone by the next check.
                    Set<Long> heldNow = threadsWaitingOnUi();
                    if (heldBefore == null) {
                        if (heldNow.isEmpty()) heldCheckAt = now + HELD_RECHECK_MS;
                        else heldBefore = heldNow;
                    } else if (heldOnBoth(heldBefore, heldNow)) {
                        report = describe(raised);
                        why = "dialog open for " + (now - lingeringSince) + " ms with a thread held behind it";
                    } else {
                        ClaudeCodeView.debug("[eclipseDialog] a dialog opened during '" + toolName
                                + "' holds no thread: taken for the user's own, not reported");
                        heldBefore = null;
                        heldCheckAt = now + HELD_RECHECK_MS;
                    }
                }
            }
            if (report == null || report.size() == 0 || call.isDone()) continue;

            String token = park(toolName, call, idsOf(report));
            ClaudeCodeView.debug("[eclipseDialog] '" + toolName + "' is waiting on a dialog after "
                    + (now - started) + " ms (" + why + "), parked as " + token + ": " + report);
            JsonObject out = new JsonObject();
            out.addProperty("dialogOpened", true);
            out.addProperty("call", token);
            out.addProperty("note", "A dialog opened while '" + toolName + "' was running and the "
                    + "call is waiting on it. Answer it with eclipseDialog (action='press'); the "
                    + "result of '" + toolName + "' comes back with that press. If the dialog is "
                    + "answered some other way, collect the result with eclipseDialog call='"
                    + token + "'.");
            out.add("dialogs", report);
            return McpToolResult.success(out);
        }
    }

    /** UI thread only. */
    private static Seen see(Display display) {
        List<Shell> shells = Dialogs.open(display);
        Set<Shell> progress = new HashSet<>();
        for (Shell shell : shells) {
            if (Dialogs.isProgress(shell)) progress.add(shell);
        }
        return new Seen(shells, Dialogs.titles(shells), progress, Dialogs.nativeIdsOf(shells));
    }

    /**
     * The dialogs a call may be waiting on: those open now that were not open before it
     * ({@code before}, null when that look never came back) and that ask something rather
     * than report progress.
     */
    static <S> List<S> raisedBy(List<S> open, List<S> before, Set<S> progress) {
        List<S> raised = new ArrayList<>();
        for (S shell : open) {
            boolean alreadyOpen = before != null && before.contains(shell);
            if (!alreadyOpen && !progress.contains(shell)) raised.add(shell);
        }
        return raised;
    }

    /** Null when the UI thread does not answer in time. */
    private static Seen look(Display display) {
        return Dialogs.onUi(() -> see(display), UI_ANSWER_MS);
    }

    /** What was open before the call; null while the UI thread has not got to it. */
    private static Seen seenBefore(CompletableFuture<Seen> before) {
        return before.isCompletedExceptionally() ? null : before.getNow(null);
    }

    private static JsonArray describe(List<Shell> shells) {
        if (shells.isEmpty()) return new JsonArray();
        return Dialogs.onUi(() -> Dialogs.describeAll(shells), UI_ANSWER_MS);
    }

    private static JsonArray nativeDialogsHere() {
        return NativeDialogs.list(true).getAsJsonArray("dialogs");
    }

    static Set<String> idsOf(JsonArray dialogs) {
        Set<String> ids = new HashSet<>();
        for (JsonElement dialog : dialogs) {
            JsonObject j = dialog.getAsJsonObject();
            if (j.has("id")) ids.add(j.get("id").getAsString());
        }
        return ids;
    }

    /**
     * Whether a thread is parked in {@code Display.syncExec}, waiting for the UI thread to
     * finish its runnable. While the UI thread is answering other requests at the same time,
     * that runnable can only be sitting in a nested event loop: a dialog it opened.
     */
    static boolean waitingOnUiThread(StackTraceElement[] stack) {
        for (StackTraceElement frame : stack) {
            if ("syncExec".equals(frame.getMethodName())
                    && ("org.eclipse.swt.widgets.Synchronizer".equals(frame.getClassName())
                            || "org.eclipse.swt.widgets.Display".equals(frame.getClassName()))) {
                return true;
            }
        }
        return false;
    }

    /** The ids of the threads inside {@code syncExec} at this moment. */
    private static Set<Long> threadsWaitingOnUi() {
        Set<Long> ids = new HashSet<>();
        for (Map.Entry<Thread, StackTraceElement[]> entry : Thread.getAllStackTraces().entrySet()) {
            if (waitingOnUiThread(entry.getValue())) ids.add(entry.getKey().threadId());
        }
        return ids;
    }

    /** Whether one and the same thread was waiting at both moments. */
    static boolean heldOnBoth(Set<Long> before, Set<Long> now) {
        return !Collections.disjoint(before, now);
    }

    // ── Calls left running behind a dialog ──────────────────────────────────────────

    /** Parks a call behind the dialogs it was reported with; returns the token for it. */
    static String park(String tool, Future<McpToolResult> call, Set<String> dialogs) {
        String token = "call-" + UUID.randomUUID().toString().substring(0, 8);
        synchronized (PARKED) {
            if (PARKED.size() >= MAX_PARKED) PARKED.remove(0);
            PARKED.add(new Parked(token, tool, call, new HashSet<>(dialogs)));
        }
        return token;
    }

    private static boolean wanted(Parked parked, String dialog, String token) {
        return (token != null && token.equals(parked.token()))
                || (dialog != null && parked.dialogs().contains(dialog));
    }

    /**
     * The results of the finished calls that were parked behind {@code dialog} or carry
     * {@code token} (either may be null), each handed out once, as {@code [{call, tool,
     * result}]}. Waits up to {@code waitMs} for the first one while such a call is still
     * running. Asked with neither, it has nothing to give: whose call it is cannot be known.
     */
    static JsonArray takeFinished(String dialog, String token, long waitMs) {
        long deadline = System.currentTimeMillis() + waitMs;
        while (true) {
            JsonArray finished = new JsonArray();
            boolean anyRunning = false;
            synchronized (PARKED) {
                for (Iterator<Parked> it = PARKED.iterator(); it.hasNext();) {
                    Parked parked = it.next();
                    if (!wanted(parked, dialog, token)) continue;
                    if (!parked.call().isDone()) {
                        anyRunning = true;
                        continue;
                    }
                    it.remove();
                    JsonObject j = new JsonObject();
                    j.addProperty("call", parked.token());
                    j.addProperty("tool", parked.tool());
                    j.add("result", resultOf(parked.call()));
                    finished.add(j);
                }
            }
            if (finished.size() > 0 || !anyRunning || System.currentTimeMillis() >= deadline) {
                return finished;
            }
            try {
                Thread.sleep(100);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return finished;
            }
        }
    }

    /**
     * A press on {@code dialog} brought {@code opened} up: a wizard's next page, a follow-up
     * question. The calls still waiting behind the first are waiting behind those now.
     */
    static void follow(String dialog, Set<String> opened) {
        if (dialog == null || opened.isEmpty()) return;
        synchronized (PARKED) {
            for (Parked parked : PARKED) {
                if (parked.dialogs().contains(dialog) && !parked.call().isDone()) {
                    parked.dialogs().addAll(opened);
                }
            }
        }
    }

    /** The tools whose calls are still parked behind a dialog. */
    static JsonArray stillParked() {
        return parkedTools(false);
    }

    /** The tools whose parked calls have finished and whose results nobody has collected. */
    static JsonArray finishedUncollected() {
        return parkedTools(true);
    }

    private static JsonArray parkedTools(boolean finished) {
        JsonArray names = new JsonArray();
        synchronized (PARKED) {
            for (Parked parked : PARKED) {
                if (parked.call().isDone() == finished) names.add(parked.tool());
            }
        }
        return names;
    }

    /** Whether a call that has not finished is parked behind {@code dialog}. */
    static boolean hasCallBehind(String dialog) {
        if (dialog == null) return false;
        synchronized (PARKED) {
            for (Parked parked : PARKED) {
                if (parked.dialogs().contains(dialog) && !parked.call().isDone()) return true;
            }
        }
        return false;
    }

    /** For tests: the list is the one static thing here. */
    static void forgetParked() {
        synchronized (PARKED) {
            PARKED.clear();
        }
    }

    private static JsonObject resultOf(Future<McpToolResult> call) {
        JsonObject j = new JsonObject();
        try {
            McpToolResult result = call.get();
            if (result == null) {
                j.addProperty("error", "Tool returned no result.");
                return j;
            }
            StringBuilder text = new StringBuilder();
            result.content().forEach(block -> {
                if (text.length() > 0) text.append('\n');
                text.append(block.getAsJsonObject().get("text").getAsString());
            });
            if (result.isError()) {
                j.addProperty("error", text.toString());
                return j;
            }
            try {
                j.add("value", JsonParser.parseString(text.toString()));
            } catch (RuntimeException notJson) {
                j.addProperty("value", text.toString());
            }
        } catch (Exception e) {
            Throwable cause = e instanceof ExecutionException && e.getCause() != null ? e.getCause() : e;
            j.addProperty("error", cause.getClass().getSimpleName() + ": " + cause.getMessage());
        }
        return j;
    }
}
