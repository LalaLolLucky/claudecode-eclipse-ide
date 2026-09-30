package com.anthropic.claudecode.eclipse.tools;

import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.CoreException;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.ltk.core.refactoring.CheckConditionsOperation;
import org.eclipse.ltk.core.refactoring.CreateChangeOperation;
import org.eclipse.ltk.core.refactoring.PerformChangeOperation;
import org.eclipse.ltk.core.refactoring.Refactoring;
import org.eclipse.ltk.core.refactoring.RefactoringCore;
import org.eclipse.ltk.core.refactoring.RefactoringStatus;
import org.eclipse.ltk.core.refactoring.RefactoringStatusEntry;

import com.google.gson.JsonArray;

/**
 * Runs an LTK refactoring the way the refactoring wizard does, minus the dialog: check all
 * conditions, create the change only if they pass, perform it as one workspace operation,
 * and record it with the refactoring undo manager so Edit › Undo offers it.
 *
 * <p>The threshold is the one point where the wizard would stop and ask: by default an
 * ERROR-level condition fails the refactoring and nothing is changed; {@code proceedPastErrors}
 * moves the threshold to FATAL. Shared by {@link RefactorResourceTool} and the JDT
 * {@code refactorJava} tool; needs only {@code org.eclipse.ltk.core.refactoring}.
 */
public final class RefactoringRunner {

    private RefactoringRunner() {}

    /** What happened: whether the change ran, and the statuses that explain why or why not. */
    public record Outcome(boolean performed, RefactoringStatus conditions, RefactoringStatus validation) {
    }

    public static Outcome run(Refactoring refactoring, boolean proceedPastErrors) throws CoreException {
        CheckConditionsOperation check = new CheckConditionsOperation(refactoring,
                CheckConditionsOperation.ALL_CONDITIONS);
        CreateChangeOperation create = new CreateChangeOperation(check,
                proceedPastErrors ? RefactoringStatus.FATAL : RefactoringStatus.ERROR);
        PerformChangeOperation perform = new PerformChangeOperation(create);
        perform.setUndoManager(RefactoringCore.getUndoManager(), refactoring.getName());
        ResourcesPlugin.getWorkspace().run(perform, new NullProgressMonitor());
        return new Outcome(perform.changeExecuted() && !perform.changeExecutionFailed(),
                create.getConditionCheckingStatus(), perform.getValidationStatus());
    }

    /** Each status entry as {@code "SEVERITY: message"}. */
    public static JsonArray messages(RefactoringStatus status) {
        JsonArray arr = new JsonArray();
        if (status == null) return arr;
        for (RefactoringStatusEntry entry : status.getEntries()) {
            String severity = entry.isFatalError() ? "FATAL" : entry.isError() ? "ERROR"
                    : entry.isWarning() ? "WARNING" : "INFO";
            arr.add(severity + ": " + entry.getMessage());
        }
        return arr;
    }

    /** A headline followed by the status entries, one per line. */
    public static String describe(String headline, RefactoringStatus... statuses) {
        StringBuilder sb = new StringBuilder(headline);
        for (RefactoringStatus status : statuses) {
            JsonArray msgs = messages(status);
            for (int i = 0; i < msgs.size(); i++) sb.append("\n- ").append(msgs.get(i).getAsString());
        }
        return sb.toString();
    }
}
