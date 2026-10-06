package com.anthropic.claudecode.eclipse.tools;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.WeakHashMap;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.function.Supplier;

import org.eclipse.jface.dialogs.Dialog;
import org.eclipse.jface.dialogs.ProgressMonitorDialog;
import org.eclipse.swt.SWT;
import org.eclipse.swt.custom.StyledText;
import org.eclipse.swt.widgets.Button;
import org.eclipse.swt.widgets.Composite;
import org.eclipse.swt.widgets.Control;
import org.eclipse.swt.widgets.Display;
import org.eclipse.swt.widgets.Label;
import org.eclipse.swt.widgets.Link;
import org.eclipse.swt.widgets.Shell;
import org.eclipse.swt.widgets.Text;
import org.eclipse.ui.IWorkbenchWindow;
import org.eclipse.ui.PlatformUI;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Reads the dialogs this Eclipse has open: which shells count as one, and each one's title,
 * text and buttons. Shared by the {@code eclipseDialog} tool and {@link ToolDialogWatch}.
 *
 * <p>Only SWT widgets are seen. A permission prompt in the Claude Code view is web content,
 * not a shell, so nothing here can find or press it.
 */
final class Dialogs {

    static final String ID_PREFIX = "swt-";

    private static final int MODAL = SWT.APPLICATION_MODAL | SWT.PRIMARY_MODAL | SWT.SYSTEM_MODAL;
    private static final int MAX_TEXT_CHARS = 2000;

    private Dialogs() {
    }

    // ── UI thread access ────────────────────────────────────────────────────────────

    static Display display() {
        if (!PlatformUI.isWorkbenchRunning()) return null;
        Display display = PlatformUI.getWorkbench().getDisplay();
        return display == null || display.isDisposed() ? null : display;
    }

    /**
     * Runs {@code work} on the UI thread and waits at most {@code timeoutMs} for it. Null means
     * the UI thread did not get to it in time (it is busy, or gone); {@code work} itself must
     * not return null. Unlike {@code syncExec}, a busy UI thread cannot park the caller for
     * good — and a modal dialog is not "busy": its event loop runs this like any other.
     */
    static <T> T onUi(Supplier<T> work, long timeoutMs) {
        Display display = display();
        if (display == null) return null;
        if (Display.getCurrent() == display) return work.get();
        CompletableFuture<T> done = new CompletableFuture<>();
        try {
            display.asyncExec(() -> {
                try {
                    done.complete(work.get());
                } catch (Throwable t) {
                    done.completeExceptionally(t);
                }
            });
            return done.get(timeoutMs, TimeUnit.MILLISECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return null;
        } catch (Exception e) {
            return null;
        }
    }

    /**
     * Queues {@code work} for the UI thread and returns at once; the future completes when
     * the UI thread has run it. The queue is first in, first out, so work posted before a
     * tool call starts runs ahead of anything that call itself sends to the UI thread.
     */
    static <T> CompletableFuture<T> postUi(Supplier<T> work) {
        CompletableFuture<T> done = new CompletableFuture<>();
        Display display = display();
        if (display == null) {
            done.complete(null);
            return done;
        }
        try {
            display.asyncExec(() -> {
                try {
                    done.complete(work.get());
                } catch (Throwable t) {
                    done.completeExceptionally(t);
                }
            });
        } catch (RuntimeException disposed) {
            done.complete(null);
        }
        return done;
    }

    // ── Which shells are dialogs ────────────────────────────────────────────────────

    /**
     * When each dialog was first seen, as the number of the {@link #open} call that found
     * it: the only record there is of which of two dialogs came later. UI thread only.
     */
    private static final Map<Shell, Integer> FIRST_SEEN = new WeakHashMap<>();
    private static int looks;

    /** Open dialogs, in the display's own order. UI thread only. */
    static List<Shell> open(Display display) {
        Set<Shell> workbenchWindows = new HashSet<>();
        for (IWorkbenchWindow window : PlatformUI.getWorkbench().getWorkbenchWindows()) {
            if (window.getShell() != null) workbenchWindows.add(window.getShell());
        }
        List<Shell> dialogs = new ArrayList<>();
        int look = ++looks;
        for (Shell shell : display.getShells()) {
            if (shell.isDisposed() || !shell.isVisible() || workbenchWindows.contains(shell)) continue;
            // A JFace Dialog that is not modal (Find/Replace) still counts; a detached view or
            // a hover is neither modal nor a Dialog.
            if ((shell.getStyle() & MODAL) != 0 || shell.getData() instanceof Dialog) {
                dialogs.add(shell);
                FIRST_SEEN.putIfAbsent(shell, look);
            }
        }
        return dialogs;
    }

    /**
     * The dialogs open now that are not in {@code before} and do not merely report progress,
     * described; null when the UI thread does not answer within {@code timeoutMs}.
     */
    static JsonArray raisedSince(Set<Shell> before, long timeoutMs) {
        Display display = display();
        if (display == null) return null;
        return onUi(() -> {
            List<Shell> raised = new ArrayList<>();
            for (Shell shell : open(display)) {
                if (!before.contains(shell) && !isProgress(shell)) raised.add(shell);
            }
            return describeAll(raised);
        }, timeoutMs);
    }

    /** Just the titles, as {@code [{title}]}: all that telling a dialog from a native one needs. */
    static JsonArray titles(List<Shell> shells) {
        JsonArray out = new JsonArray();
        for (Shell shell : shells) {
            if (shell.isDisposed()) continue;
            JsonObject j = new JsonObject();
            j.addProperty("title", shell.getText());
            out.add(j);
        }
        return out;
    }

    // ── Dialogs that are the user's to answer ───────────────────────────────────────

    private static final String FOR_USER_ONLY = "com.anthropic.claudecode.eclipse.dialogForUserOnly";

    /** Marks a dialog as one that asks the person at the keyboard: listed, never pressed. */
    static void markForUserOnly(Shell shell) {
        if (shell != null && !shell.isDisposed()) shell.setData(FOR_USER_ONLY, Boolean.TRUE);
    }

    static boolean isForUserOnly(Shell shell) {
        return Boolean.TRUE.equals(shell.getData(FOR_USER_ONLY));
    }

    // What a prompt about trust is called, by Eclipse's own dialogs: p2's TrustCertificateDialog
    // ("Trust", "Do you trust these signers?", "…unsigned content…", "Always Trust Everything
    // Confirmation") and TrustAuthorityDialog, the X509CertificateViewDialog and the
    // CertificateImportWizard, and the SSH prompts ("Host Key Change", "The authenticity of
    // host … can't be established", a key's fingerprint).
    private static final String[] TRUST_IN_CLASS = { "trust", "certificate" };
    private static final String[] TRUST_IN_TITLE = { "trust", "certificate", "host key" };
    private static final String[] TRUST_IN_TEXT =
            { "do you trust", "certificate", "authenticity", "host key", "fingerprint",
                    "unsigned content", "unsigned software" };

    /**
     * Whether a dialog asks whom or what to trust — a signer, a certificate, a host key,
     * unsigned content. That answer is the user's: such a dialog is listed and never pressed.
     * Told by what the dialog is called, in its class names, its title or its text; anything
     * that only mentions one of these is left to the user too, which is the safe mistake.
     */
    static boolean asksAboutTrust(String title, String text, String classNames) {
        return mentions(classNames, TRUST_IN_CLASS) || mentions(title, TRUST_IN_TITLE)
                || mentions(text, TRUST_IN_TEXT);
    }

    private static boolean mentions(String where, String[] words) {
        if (where == null) return false;
        String lower = where.toLowerCase(java.util.Locale.ROOT);
        for (String word : words) {
            if (lower.contains(word)) return true;
        }
        return false;
    }

    /**
     * The names of what a dialog is: its class and that class's parents, and for a wizard its
     * page and the wizard itself, since every wizard is the same WizardDialog. UI thread only.
     */
    private static String classNames(Shell shell) {
        Object owner = shell.getData();
        if (owner == null) return "";
        StringBuilder names = new StringBuilder();
        for (Class<?> c = owner.getClass(); c != null && c != Object.class; c = c.getSuperclass()) {
            names.append(c.getSimpleName()).append(' ');
        }
        if (owner instanceof org.eclipse.jface.wizard.IWizardContainer container
                && container.getCurrentPage() != null) {
            names.append(container.getCurrentPage().getClass().getSimpleName()).append(' ');
            if (container.getCurrentPage().getWizard() != null) {
                names.append(container.getCurrentPage().getWizard().getClass().getSimpleName());
            }
        }
        return names.toString();
    }

    /**
     * The ids the native core lists these shells under, where it sees them as windows of
     * their own: on Windows, by window handle (see {@link NativeDialogs#idOfOwnWindow}).
     * Empty elsewhere, and for a shell whose handle cannot be read.
     */
    static Set<String> nativeIdsOf(List<Shell> shells) {
        if (!NativeDialogs.WINDOWS) return Set.of();
        Set<String> ids = new HashSet<>();
        long pid = ProcessHandle.current().pid();
        for (Shell shell : shells) {
            long handle = windowHandleOf(shell);
            if (handle != 0) ids.add(NativeDialogs.idOfOwnWindow(pid, handle));
        }
        return ids;
    }

    /** Read reflectively: the field is SWT's own on every platform, but not of one type. */
    private static long windowHandleOf(Shell shell) {
        try {
            java.lang.reflect.Field handle = Control.class.getDeclaredField("handle");
            handle.setAccessible(true);
            return handle.get(shell) instanceof Number number ? number.longValue() : 0;
        } catch (ReflectiveOperationException | RuntimeException e) {
            return 0;
        }
    }

    /** A dialog that reports on running work rather than asking a question. */
    static boolean isProgress(Shell shell) {
        Object owner = shell.getData();
        return owner instanceof ProgressMonitorDialog
                || (owner != null && "BlockedJobsDialog".equals(owner.getClass().getSimpleName()));
    }

    /**
     * The modal shell open over {@code shell}, which keeps the user from clicking it, or null.
     * SWT's own rule: an application-modal shell blocks every shell but itself and its
     * children, and a primary-modal one blocks its parent. UI thread only.
     */
    static Shell blockedBy(Shell shell) {
        Display display = shell.getDisplay();
        // Two modal dialogs that are not parent and child block each other by that rule;
        // the one in front is the one the user can click.
        Shell active = display.getActiveShell();
        if (shell == active) return null;
        for (Shell other : display.getShells()) {
            if (other == shell || other.isDisposed() || !other.isVisible()) continue;
            int style = other.getStyle();
            if ((style & (SWT.APPLICATION_MODAL | SWT.SYSTEM_MODAL)) != 0) {
                if (isWithin(shell, other)) continue;
                // With Eclipse behind another program there is no active shell to say which
                // of two such dialogs is in front, and each would read as blocked by the
                // other for good. The later one is in front; seen together, neither is held.
                if (active == null && blocksAll(shell) && !isWithin(other, shell)
                        && !inFrontOf(other, shell)) {
                    continue;
                }
                return other;
            } else if ((style & SWT.PRIMARY_MODAL) != 0 && other.getParent() == shell) {
                return other;
            }
        }
        return null;
    }

    private static boolean blocksAll(Shell shell) {
        return (shell.getStyle() & (SWT.APPLICATION_MODAL | SWT.SYSTEM_MODAL)) != 0;
    }

    /** Whether {@code a} was first seen after {@code b}. Unknown or together is "no". */
    private static boolean inFrontOf(Shell a, Shell b) {
        return laterThan(FIRST_SEEN.get(a), FIRST_SEEN.get(b));
    }

    static boolean laterThan(Integer a, Integer b) {
        return a != null && b != null && a > b;
    }

    private static boolean isWithin(Shell shell, Shell ancestor) {
        for (Composite c = shell; c != null; c = c.getParent()) {
            if (c == ancestor) return true;
        }
        return false;
    }

    /** Stable for as long as the shell lives. */
    static String idOf(Shell shell) {
        return ID_PREFIX + Integer.toHexString(System.identityHashCode(shell));
    }

    // ── Describing one ──────────────────────────────────────────────────────────────

    static JsonArray describeAll(List<Shell> shells) {
        JsonArray out = new JsonArray();
        for (Shell shell : shells) {
            if (!shell.isDisposed()) out.add(describe(shell));
        }
        return out;
    }

    /** UI thread only. */
    static JsonObject describe(Shell shell) {
        JsonObject j = new JsonObject();
        j.addProperty("id", idOf(shell));
        j.addProperty("title", shell.getText());
        if (isProgress(shell)) j.addProperty("progress", true);
        if (blockedBy(shell) != null) j.addProperty("blocked", true);

        StringBuilder text = new StringBuilder();
        JsonArray buttons = new JsonArray();
        JsonArray options = new JsonArray();
        Button defaultButton = shell.getDefaultButton();
        for (Control control : controls(shell)) {
            if (control instanceof Button button) {
                String label = plain(button.getText());
                if (label.isEmpty()) continue;
                JsonObject b = new JsonObject();
                b.addProperty("label", label);
                if ((button.getStyle() & SWT.PUSH) != 0) {
                    if (!button.isEnabled()) b.addProperty("enabled", false);
                    if (button == defaultButton) b.addProperty("default", true);
                    buttons.add(b);
                } else if ((button.getStyle() & (SWT.CHECK | SWT.RADIO)) != 0) {
                    b.addProperty("kind", (button.getStyle() & SWT.CHECK) != 0 ? "checkbox" : "radio");
                    b.addProperty("selected", button.getSelection());
                    options.add(b);
                }
            } else {
                appendText(text, readableText(control));
            }
        }
        if (text.length() > 0) j.addProperty("text", text.toString());
        // After the text is read: a prompt about trust is often told only by what it says.
        if (isForUserOnly(shell) || asksAboutTrust(shell.getText(), text.toString(), classNames(shell))) {
            j.addProperty("forUserOnly", true);
        }
        j.add("buttons", buttons);
        if (options.size() > 0) j.add("options", options);
        return j;
    }

    /** The dialog's push buttons that carry a label, in layout order. UI thread only. */
    static List<Button> pushButtons(Shell shell) {
        List<Button> buttons = new ArrayList<>();
        for (Control control : controls(shell)) {
            if (control instanceof Button button && (button.getStyle() & SWT.PUSH) != 0
                    && !plain(button.getText()).isEmpty()) {
                buttons.add(button);
            }
        }
        return buttons;
    }

    private static List<Control> controls(Composite root) {
        List<Control> found = new ArrayList<>();
        collect(root, found);
        return found;
    }

    private static void collect(Composite parent, List<Control> into) {
        for (Control child : parent.getChildren()) {
            if (child.isDisposed() || !child.getVisible()) continue;
            into.add(child);
            if (child instanceof Composite composite) collect(composite, into);
        }
    }

    private static String readableText(Control control) {
        if (control instanceof Label label) {
            return (label.getStyle() & SWT.SEPARATOR) != 0 ? "" : label.getText();
        }
        if (control instanceof Link link) return link.getText().replaceAll("</?[aA][^>]*>", "");
        if (control instanceof Text field) {
            return (field.getStyle() & SWT.READ_ONLY) != 0 ? field.getText() : "";
        }
        if (control instanceof StyledText styled) return styled.getEditable() ? "" : styled.getText();
        return "";
    }

    private static void appendText(StringBuilder text, String more) {
        if (more == null) return;
        String trimmed = more.trim();
        if (trimmed.isEmpty() || text.length() >= MAX_TEXT_CHARS) return;
        if (text.length() > 0) text.append('\n');
        int room = MAX_TEXT_CHARS - text.length();
        text.append(trimmed.length() <= room ? trimmed : trimmed.substring(0, room) + "…(truncated)");
    }

    // ── Labels ──────────────────────────────────────────────────────────────────────

    /** A button's label as it reads on screen: SWT's {@code &} mnemonic marker removed. */
    static String plain(String label) {
        if (label == null) return "";
        StringBuilder out = new StringBuilder(label.length());
        for (int i = 0; i < label.length(); i++) {
            char c = label.charAt(i);
            if (c == '&') {
                // "&&" is a literal ampersand; a single one only marks the next character.
                if (i + 1 < label.length() && label.charAt(i + 1) == '&') {
                    out.append('&');
                    i++;
                }
                continue;
            }
            out.append(c);
        }
        return out.toString().trim();
    }

    /**
     * The index of the one label that is {@code wanted}, ignoring case; -1 when none is, -2
     * when several are. Never a prefix or a guess: the caller names the button in full.
     */
    static int match(List<String> labels, String wanted) {
        String target = wanted == null ? "" : wanted.trim();
        int found = -1;
        for (int i = 0; i < labels.size(); i++) {
            if (labels.get(i).equalsIgnoreCase(target)) {
                if (found >= 0) return -2;
                found = i;
            }
        }
        return found;
    }
}
