package com.anthropic.claudecode.eclipse.ui;

import org.eclipse.e4.ui.model.application.ui.advanced.MPlaceholder;
import org.eclipse.e4.ui.model.application.ui.basic.MPart;
import org.eclipse.e4.ui.model.application.ui.basic.MWindow;
import org.eclipse.e4.ui.workbench.modeling.EModelService;
import org.eclipse.swt.widgets.Display;
import org.eclipse.ui.IPartListener2;
import org.eclipse.ui.IViewReference;
import org.eclipse.ui.IWindowListener;
import org.eclipse.ui.IWorkbench;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.IWorkbenchPartReference;
import org.eclipse.ui.IWorkbenchWindow;
import org.eclipse.ui.PartInitException;
import org.eclipse.ui.PlatformUI;

import com.anthropic.claudecode.eclipse.Activator;
import com.anthropic.claudecode.eclipse.Constants;

/**
 * Helpers for "Exclusively use terminal" ({@link Constants#PREF_TERMINAL_ONLY}).
 *
 * <p>What the user can see is decided declaratively, the way the debug-only UI is (see
 * {@link DebugModeUi}): the Claude Code view's activity in plugin.xml has an
 * {@code enabledWhen} bound to the {@code codeViewEnabled} source variable
 * ({@link DebugModeSourceProvider}), which takes the view out of Show View and Show In and
 * its commands out of the key bindings, and the two menu entries carry a
 * {@code visibleWhen} on the same variable.
 *
 * <p>Two things are left to code.
 *
 * <p>An open view has to be closed — in every perspective, not only the one in front. A
 * view the workbench finds in its layout while the activity is off cannot be created at all
 * (its descriptor is filtered, so the workbench puts an error part in its place), which is
 * why none may be left in any perspective for the next start to find.
 *
 * <p>And an activity only filters what is offered: {@code IWorkbenchPage.showView} still
 * opens a filtered view for anyone who asks by id (a key the user bound, another plug-in).
 * So whatever still gets as far as showing the view is closed again and sent to the Claude
 * Terminal instead.
 */
public final class TerminalOnlyUi {

    /** UI thread only. */
    private static boolean installed;

    /** One redirect at a time: "opened" and "visible" both report a view that was shown. */
    private static boolean redirecting;

    private TerminalOnlyUi() {
    }

    public static boolean isOn() {
        Activator activator = Activator.getDefault();
        if (activator == null) return false;
        return activator.getPreferenceStore().getBoolean(Constants.PREF_TERMINAL_ONLY);
    }

    /**
     * Starts watching every window for the Claude Code view being shown while the terminal
     * is used exclusively, and closes one a start with the preference already on finds
     * open. Safe to call more than once and from any thread.
     */
    static void install() {
        Display.getDefault().asyncExec(() -> {
            if (installed || !PlatformUI.isWorkbenchRunning()) return;
            installed = true;
            IWorkbench workbench = PlatformUI.getWorkbench();
            for (IWorkbenchWindow window : workbench.getWorkbenchWindows()) watch(window);
            workbench.addWindowListener(new IWindowListener() {
                @Override
                public void windowOpened(IWorkbenchWindow window) {
                    watch(window);
                }

                @Override
                public void windowActivated(IWorkbenchWindow window) {
                    watch(window);
                }

                @Override
                public void windowDeactivated(IWorkbenchWindow window) {
                }

                @Override
                public void windowClosed(IWorkbenchWindow window) {
                }
            });
            if (isOn()) closeCodeViewIfOpen();
        });
    }

    /** Adding a listener a window already has does nothing, so this may be repeated. */
    private static void watch(IWorkbenchWindow window) {
        window.getPartService().addPartListener(SHOWN);
    }

    /**
     * Whether the Claude Code view is open: in the perspective in front of any window, or
     * alive in a perspective that is not. UI thread.
     */
    public static boolean isCodeViewOpen() {
        if (ClaudeGuiView.liveInstance() != null) return true;
        if (!PlatformUI.isWorkbenchRunning()) return false;
        for (IWorkbenchWindow window : PlatformUI.getWorkbench().getWorkbenchWindows()) {
            for (IWorkbenchPage page : window.getPages()) {
                if (page.findViewReference(ClaudeGuiView.VIEW_ID) != null) return true;
            }
        }
        return false;
    }

    /**
     * Closes the Claude Code view wherever it is open, in every perspective of every
     * window, which ends the conversations running in it. UI thread.
     */
    public static void closeCodeViewIfOpen() {
        if (!PlatformUI.isWorkbenchRunning()) return;
        IWorkbenchWindow[] windows = PlatformUI.getWorkbench().getWorkbenchWindows();
        for (IWorkbenchWindow window : windows) {
            for (IWorkbenchPage page : window.getPages()) {
                IViewReference ref = page.findViewReference(ClaudeGuiView.VIEW_ID);
                if (ref != null) {
                    ClaudeCodeView.debug("[terminal-only] closing the Claude Code view");
                    page.hideView(ref);
                }
            }
        }
        // A page only knows the views of the perspective in front, and hiding one takes it
        // out of that perspective alone. A view that is also open in another perspective is
        // the same part, still alive there with its conversations; asked for by the part
        // itself, the page takes it down for all of them.
        ClaudeGuiView elsewhere = ClaudeGuiView.liveInstance();
        if (elsewhere != null) {
            try {
                ClaudeCodeView.debug("[terminal-only] closing the Claude Code view left open in another perspective");
                elsewhere.getSite().getPage().hideView(elsewhere);
            } catch (RuntimeException e) {
                Activator.logError("Failed to close the Claude Code view in another perspective", e);
            }
        }
        // What is left is layout only: the view's place in perspectives not shown since it
        // was last open there. Left as it is, the next start would find it — see the class
        // comment for what the workbench makes of that.
        for (IWorkbenchWindow window : windows) takeOutOfEveryPerspective(window);
    }

    /**
     * Marks the view's place in each perspective of {@code window} as closed, the state a
     * view the user closed is left in: the place is kept, so the view comes back where it
     * was once it is opened again, and nothing is shown there meanwhile. The same set the
     * workbench goes through at startup when it gives every view in the layout its
     * reference (rendered view placeholders, inside and outside the perspectives).
     */
    private static void takeOutOfEveryPerspective(IWorkbenchWindow window) {
        try {
            MWindow model = window.getService(MWindow.class);
            EModelService modelService = window.getService(EModelService.class);
            if (model == null || modelService == null) return;
            for (MPlaceholder placeholder : modelService.findElements(model, null, MPlaceholder.class, null,
                    EModelService.IN_ANY_PERSPECTIVE | EModelService.OUTSIDE_PERSPECTIVE)) {
                if (placeholder.isToBeRendered() && placeholder.getRef() instanceof MPart part
                        && ClaudeGuiView.VIEW_ID.equals(part.getElementId())) {
                    ClaudeCodeView.debug("[terminal-only] taking the Claude Code view out of a perspective's layout");
                    placeholder.setToBeRendered(false);
                }
            }
        } catch (RuntimeException e) {
            // The watch on each window still closes the view the moment it is shown.
            Activator.logError("Failed to take the Claude Code view out of every perspective", e);
        }
    }

    /** Opens the Claude Terminal, with a session in it, in place of the Claude Code view. */
    public static ClaudeCliView openTerminal(IWorkbenchPage page) throws PartInitException {
        Activator activator = Activator.getDefault();
        if (!activator.isServerRunning()) activator.initialize();
        ClaudeCliView view = (ClaudeCliView) page.showView(ClaudeCliView.VIEW_ID);
        if (view != null) view.ensureAtLeastOneTab();
        return view;
    }

    /**
     * The Claude Code view was shown while the terminal is used exclusively: close it and
     * open the Claude Terminal in its place. By id, so it also answers for the error part
     * the workbench shows when it cannot create the view.
     */
    private static void shown(IWorkbenchPartReference ref) {
        if (!ClaudeGuiView.VIEW_ID.equals(ref.getId()) || redirecting || !isOn()) return;
        redirecting = true;
        ClaudeCodeView.debug("[terminal-only] the Claude Code view was shown; sending to the Claude Terminal");
        // Deferred: a part cannot be hidden from inside its own opening.
        Display.getDefault().asyncExec(() -> {
            try {
                IWorkbenchPage page = ref.getPage();
                if (page == null) return;
                if (ref instanceof IViewReference viewRef) page.hideView(viewRef);
                openTerminal(page);
            } catch (Exception e) {
                Activator.logError("Failed to open the Claude Terminal in place of the Claude Code view", e);
            } finally {
                redirecting = false;
            }
        });
    }

    /** Every method spelled out: IPartListener2's are default methods only on newer platforms. */
    private static final IPartListener2 SHOWN = new IPartListener2() {
        @Override
        public void partOpened(IWorkbenchPartReference ref) {
            shown(ref);
        }

        @Override
        public void partVisible(IWorkbenchPartReference ref) {
            shown(ref);
        }

        @Override
        public void partActivated(IWorkbenchPartReference ref) {
        }

        @Override
        public void partBroughtToTop(IWorkbenchPartReference ref) {
        }

        @Override
        public void partClosed(IWorkbenchPartReference ref) {
        }

        @Override
        public void partDeactivated(IWorkbenchPartReference ref) {
        }

        @Override
        public void partHidden(IWorkbenchPartReference ref) {
        }

        @Override
        public void partInputChanged(IWorkbenchPartReference ref) {
        }
    };
}
