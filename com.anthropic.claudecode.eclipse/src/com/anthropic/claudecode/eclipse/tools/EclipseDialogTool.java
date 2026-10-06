package com.anthropic.claudecode.eclipse.tools;

import java.util.ArrayList;
import java.util.List;
import java.util.Set;

import org.eclipse.swt.SWT;
import org.eclipse.swt.widgets.Button;
import org.eclipse.swt.widgets.Display;
import org.eclipse.swt.widgets.Event;
import org.eclipse.swt.widgets.Shell;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

/**
 * Lists the dialogs that are open and presses a button in one — what clicking the button
 * does. Eclipse's own dialogs are found as SWT shells and buttons, so any plug-in's are
 * reachable; native dialogs and those of other Eclipse instances come from the native core
 * ({@link NativeDialogs}).
 *
 * <p>A button is pressed only when named in full; nothing here picks a default. Relies only
 * on SWT and JFace, re-exported by {@code org.eclipse.ui}, a hard {@code Require-Bundle} — no
 * availability guard.
 */
public class EclipseDialogTool implements McpTool {

    private static final long UI_ANSWER_MS = 2000;
    /** How long a press waits to see its dialog close. */
    private static final long CLOSE_WAIT_MS = 1500;
    /** How long a press waits for a call that was parked behind the dialog to finish. */
    private static final long RESUME_WAIT_MS = 10_000;

    @Override
    public String toolName() {
        return "eclipseDialog";
    }

    @Override
    public String description() {
        return "See and answer the dialogs that are open — a confirmation, an error, a wizard, a "
                + "file chooser — in this Eclipse and in other Eclipse instances, such as one "
                + "started with 'runAs'. action='list' (default) shows each dialog's id, title, "
                + "text and buttons. action='press' clicks the button whose label is 'button', "
                + "exactly as listed; name the dialog with 'dialog' when more than one is open. "
                + "Always list first and read the dialog before pressing: a press is the same as "
                + "the user's click and cannot be undone. When another tool reports "
                + "'dialogOpened', its call is still waiting on that dialog; after the press its "
                + "result is returned here under 'finishedCalls'. If that dialog was answered "
                + "some other way, pass the 'call' value that tool reported to collect the "
                + "result. A dialog listed with 'forUserOnly' is the user's to answer.";
    }

    /**
     * Marks a dialog as one that asks the person at the keyboard, such as this plug-in's own
     * confirmations: it is still listed, and no press reaches it. Call between
     * {@code create()} and {@code open()}.
     */
    public static void forUserOnly(Shell shell) {
        Dialogs.markForUserOnly(shell);
    }

    @Override
    public JsonObject inputSchema() {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");

        JsonObject props = new JsonObject();

        JsonObject action = new JsonObject();
        action.addProperty("type", "string");
        action.addProperty("description", "list (default) or press");
        JsonArray actions = new JsonArray();
        actions.add("list");
        actions.add("press");
        action.add("enum", actions);
        props.add("action", action);

        JsonObject button = new JsonObject();
        button.addProperty("type", "string");
        button.addProperty("description",
                "For 'press': the button's full label as shown by action='list', e.g. 'Cancel'.");
        props.add("button", button);

        JsonObject dialog = new JsonObject();
        dialog.addProperty("type", "string");
        dialog.addProperty("description",
                "For 'press': the dialog's id as shown by action='list'. Required when more than "
                        + "one dialog is open.");
        props.add("dialog", dialog);

        JsonObject call = new JsonObject();
        call.addProperty("type", "string");
        call.addProperty("description",
                "The 'call' value another tool reported with 'dialogOpened': returns that call's "
                        + "result once it has finished, whoever answered the dialog.");
        props.add("call", call);

        schema.add("properties", props);
        return schema;   // no args means "list"
    }

    @Override
    public McpToolResult execute(JsonObject params) {
        try {
            String action = str(params, "action");
            String button = str(params, "button");
            String dialog = str(params, "dialog");
            String call = str(params, "call");
            ClaudeCodeView.debug("[eclipseDialog] action=" + action + " dialog=" + dialog
                    + " button=" + button + " call=" + call);

            if (action == null || "list".equalsIgnoreCase(action)) return list(call);
            if ("press".equalsIgnoreCase(action)) {
                if (button == null) return McpToolResult.error("'button' is required for action='press'.");
                return press(dialog, button, call);
            }
            return McpToolResult.error("Unknown action: " + action + ". Use list or press.");
        } catch (Exception e) {
            ClaudeCodeView.debug("[eclipseDialog] failed: " + e);
            return McpToolResult.error("eclipseDialog failed: " + e.getClass().getSimpleName()
                    + ": " + e.getMessage());
        }
    }

    // ── Actions ─────────────────────────────────────────────────────────────────────

    private McpToolResult list(String call) {
        JsonObject out = new JsonObject();
        JsonArray dialogs = openDialogs(out);
        out.addProperty("count", dialogs.size());
        out.add("dialogs", dialogs);
        addParkedCalls(out, null, call, 0);
        ClaudeCodeView.debug("[eclipseDialog] list → " + dialogs.size());
        return McpToolResult.success(out);
    }

    private McpToolResult press(String dialogId, String wanted, String call) {
        String id = dialogId;
        // What is open before the press is listed only when it is needed: to find the one
        // dialog there is, or to tell afterwards what a press with a call waiting behind it
        // brought up. Otherwise a press costs no listing beyond its own.
        JsonArray open = null;
        if (id == null) {
            open = openDialogs(new JsonObject());
            if (open.size() == 0) return McpToolResult.error("No dialog is open.");
            if (open.size() > 1) {
                return McpToolResult.error(open.size() + " dialogs are open. Name one with 'dialog': "
                        + open);
            }
            id = open.get(0).getAsJsonObject().get("id").getAsString();
        } else if (ToolDialogWatch.hasCallBehind(id)) {
            open = openDialogs(new JsonObject());
        }
        Pressed done = NativeDialogs.isNativeId(id) ? pressNative(id, wanted) : pressSwt(id, wanted);
        if (done.problem() != null) return McpToolResult.error(done.problem());
        return pressed(done, id, call, open == null ? null : ToolDialogWatch.idsOf(open));
    }

    /** A press that went through, or why it did not. */
    private record Pressed(String label, String title, boolean closed, String problem) {
        static Pressed failed(String problem) {
            return new Pressed(null, null, false, problem);
        }
    }

    /** What the UI thread decided about a press: the shell it went to, or why not. */
    private record Aimed(Shell shell, String title, String label, String problem) {
    }

    private Pressed pressSwt(String dialogId, String wanted) {
        Display display = Dialogs.display();
        if (display == null) return Pressed.failed("The workbench is not available.");

        Aimed aimed = Dialogs.onUi(() -> aim(display, dialogId, wanted), UI_ANSWER_MS);
        if (aimed == null) return Pressed.failed(UI_BUSY);
        if (aimed.problem() != null) return Pressed.failed(aimed.problem());

        Shell shell = aimed.shell();
        boolean closed = waitUntil(() -> Boolean.TRUE.equals(Dialogs.onUi(shell::isDisposed, UI_ANSWER_MS)));
        return new Pressed(aimed.label(), aimed.title(), closed, null);
    }

    /** Finds the dialog and its button and queues the click. UI thread only. */
    private static Aimed aim(Display display, String dialogId, String wanted) {
        Shell shell = null;
        for (Shell candidate : Dialogs.open(display)) {
            if (dialogId.equals(Dialogs.idOf(candidate))) shell = candidate;
        }
        if (shell == null) {
            return problem("No open dialog has id '" + dialogId + "'. Use action='list' to see "
                    + "the dialogs open now.");
        }
        // describe, not the marker alone: it also knows a prompt about trust when it reads one.
        if (Dialogs.describe(shell).has("forUserOnly")) {
            ClaudeCodeView.debug("[eclipseDialog] press refused: '" + shell.getText() + "' is for the user only");
            return problem("'" + shell.getText() + "' asks the user, and is left for them to answer.");
        }
        Shell over = Dialogs.blockedBy(shell);
        if (over != null) {
            return problem("'" + shell.getText() + "' is waiting on '" + over.getText()
                    + "', which is open over it. Answer that one first.");
        }

        List<Button> buttons = Dialogs.pushButtons(shell);
        List<String> labels = new ArrayList<>();
        for (Button candidate : buttons) labels.add(Dialogs.plain(candidate.getText()));
        int index = Dialogs.match(labels, wanted);
        if (index == -2) {
            return problem("More than one button in '" + shell.getText() + "' is labelled '"
                    + wanted + "'.");
        }
        if (index < 0) {
            return problem("'" + shell.getText() + "' has no button labelled '" + wanted
                    + "'. Its buttons: " + String.join(", ", labels));
        }
        Button button = buttons.get(index);
        if (!button.isEnabled()) {
            return problem("The '" + labels.get(index) + "' button in '" + shell.getText()
                    + "' is disabled.");
        }

        // Queued rather than run here: a button that opens a further dialog would otherwise
        // hold this call inside that dialog's event loop.
        display.asyncExec(() -> {
            if (!button.isDisposed() && button.isEnabled()) button.notifyListeners(SWT.Selection, new Event());
        });
        return new Aimed(shell, shell.getText(), labels.get(index), null);
    }

    private Pressed pressNative(String dialogId, String wanted) {
        // Looked up first: a prompt about trust is the user's in another Eclipse as in this one,
        // and the native core presses whatever it is told to.
        for (JsonElement listed : NativeDialogs.list(false).getAsJsonArray("dialogs")) {
            JsonObject dialog = listed.getAsJsonObject();
            if (dialogId.equals(dialog.get("id").getAsString()) && dialog.has("forUserOnly")) {
                String title = dialog.has("title") ? dialog.get("title").getAsString() : dialogId;
                ClaudeCodeView.debug("[eclipseDialog] press refused: '" + title + "' is for the user only");
                return Pressed.failed("'" + title + "' asks the user, and is left for them to answer.");
            }
        }
        JsonObject reply = NativeDialogs.press(dialogId, wanted);
        if (reply.has("error")) return Pressed.failed(reply.get("error").getAsString());
        boolean closed = waitUntil(() -> {
            for (JsonElement dialog : NativeDialogs.list(false).getAsJsonArray("dialogs")) {
                if (dialogId.equals(dialog.getAsJsonObject().get("id").getAsString())) return false;
            }
            return true;
        });
        return new Pressed(reply.get("pressed").getAsString(), reply.get("dialog").getAsString(), closed, null);
    }

    /**
     * The reply to a press that went through. {@code openBefore} is the ids of the dialogs
     * that were open before it, when that was looked at: what has come up since is the
     * pressed dialog's doing.
     */
    private McpToolResult pressed(Pressed pressed, String dialogId, String call, Set<String> openBefore) {
        JsonObject out = new JsonObject();
        out.addProperty("pressed", pressed.label());
        out.addProperty("dialog", pressed.title());
        out.addProperty("closed", pressed.closed());

        JsonArray now = openDialogs(new JsonObject());
        if (openBefore != null) {
            Set<String> opened = ToolDialogWatch.idsOf(now);
            opened.removeAll(openBefore);
            ToolDialogWatch.follow(dialogId, opened);
        }

        addParkedCalls(out, dialogId, call, RESUME_WAIT_MS);
        if (now.size() > 0) out.add("dialogs", now);
        ClaudeCodeView.debug("[eclipseDialog] pressed '" + pressed.label() + "' in '" + pressed.title()
                + "' closed=" + pressed.closed());
        return McpToolResult.success(out);
    }

    // ── Helpers ─────────────────────────────────────────────────────────────────────

    private static final String UI_BUSY = "Eclipse's UI thread did not answer within "
            + (UI_ANSWER_MS / 1000) + " s — it is busy. Try again.";

    /**
     * Every dialog that is open: this Eclipse's SWT ones, then the native core's. Why either
     * half is missing, when it is, goes into {@code notes}.
     */
    private static JsonArray openDialogs(JsonObject notes) {
        Display display = Dialogs.display();
        // Where the native core reports this Eclipse's own shells as well (Windows), these
        // are the ids it does so under, so that each is listed once.
        java.util.concurrent.atomic.AtomicReference<Set<String>> ownShells =
                new java.util.concurrent.atomic.AtomicReference<>(Set.of());
        JsonArray swt = display == null ? null : Dialogs.onUi(() -> {
            List<Shell> shells = Dialogs.open(display);
            ownShells.set(Dialogs.nativeIdsOf(shells));
            return Dialogs.describeAll(shells);
        }, UI_ANSWER_MS);
        if (swt == null) {
            notes.addProperty("eclipseDialogsUnavailable", UI_BUSY);
            swt = new JsonArray();
        }
        JsonObject natives = NativeDialogs.list(false);
        if (natives.has("unavailable")) {
            notes.addProperty("otherDialogsUnavailable", natives.get("unavailable").getAsString());
        }
        JsonArray all = new JsonArray();
        all.addAll(swt);
        all.addAll(NativeDialogs.withoutThoseIn(swt, natives.getAsJsonArray("dialogs"), ownShells.get()));
        return all;
    }

    private interface Check {
        boolean holds();
    }

    private static boolean waitUntil(Check closed) {
        long deadline = System.currentTimeMillis() + CLOSE_WAIT_MS;
        while (true) {
            if (closed.holds()) return true;
            if (System.currentTimeMillis() >= deadline) return false;
            try {
                Thread.sleep(150);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return false;
            }
        }
    }

    private static Aimed problem(String message) {
        return new Aimed(null, null, null, message);
    }

    /**
     * Adds the results of the calls that were parked behind {@code dialogId} or are named by
     * {@code call}, waiting up to {@code waitMs} for one of those. Other parked calls are
     * named, not answered for: they may be another conversation's.
     */
    private static void addParkedCalls(JsonObject out, String dialogId, String call, long waitMs) {
        JsonArray finished = ToolDialogWatch.takeFinished(dialogId, call, waitMs);
        if (finished.size() > 0) out.add("finishedCalls", finished);
        JsonArray waiting = ToolDialogWatch.stillParked();
        if (waiting.size() > 0) out.add("callsStillWaiting", waiting);
        // Named so a finished call is not simply gone from view; its result still needs
        // the 'call' value its own caller was given.
        JsonArray uncollected = ToolDialogWatch.finishedUncollected();
        if (uncollected.size() > 0) out.add("callsFinishedUncollected", uncollected);
    }

    private static String str(JsonObject params, String key) {
        if (!params.has(key) || params.get(key).isJsonNull()) return null;
        String v = params.get(key).getAsString().trim();
        return v.isEmpty() ? null : v;
    }
}
