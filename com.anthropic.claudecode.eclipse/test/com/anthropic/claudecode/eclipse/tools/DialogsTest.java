package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Future;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Tests for the parts of the dialog tooling that need no display: how a button label reads,
 * which label a press means, and how a call stuck behind a dialog is recognised.
 */
class DialogsTest {

    @Test
    void aMnemonicMarkerIsNotPartOfTheLabel() {
        assertEquals("Yes", Dialogs.plain("&Yes"));
        assertEquals("Run in Background", Dialogs.plain("Run in &Background"));
        assertEquals("Next >", Dialogs.plain("  &Next >  "));
    }

    @Test
    void aDoubledAmpersandIsALiteralOne() {
        assertEquals("Save & Launch", Dialogs.plain("Save && &Launch"));
    }

    @Test
    void aMissingLabelIsEmpty() {
        assertEquals("", Dialogs.plain(null));
        assertEquals("", Dialogs.plain("&"));
    }

    @Test
    void aPressMeansTheOneButtonWithThatLabelWhateverItsCase() {
        assertEquals(1, Dialogs.match(List.of("Proceed", "Cancel"), "cancel"));
        assertEquals(0, Dialogs.match(List.of("Proceed", "Cancel"), " Proceed "));
    }

    @Test
    void aPartOfALabelMatchesNothing() {
        assertEquals(-1, Dialogs.match(List.of("Proceed", "Cancel"), "Proc"));
        assertEquals(-1, Dialogs.match(List.of("Proceed", "Cancel"), ""));
        assertEquals(-1, Dialogs.match(List.of("Proceed", "Cancel"), null));
    }

    @Test
    void twoButtonsWithTheSameLabelAreAmbiguous() {
        assertEquals(-2, Dialogs.match(List.of("Browse...", "Browse...", "OK"), "Browse..."));
    }

    @Test
    void aThreadInsideSyncExecIsWaitingOnTheUiThread() {
        StackTraceElement[] stack = {
                new StackTraceElement("java.lang.Object", "wait", "Object.java", 1),
                new StackTraceElement("org.eclipse.swt.widgets.Synchronizer", "syncExec", "Synchronizer.java", 2),
                new StackTraceElement("org.eclipse.swt.widgets.Display", "syncExec", "Display.java", 3),
                new StackTraceElement("com.anthropic.claudecode.eclipse.tools.RunAsTool", "launchElement", "RunAsTool.java", 4),
        };
        assertTrue(ToolDialogWatch.waitingOnUiThread(stack));
    }

    @Test
    void aThreadDoingItsOwnWorkIsNotWaitingOnTheUiThread() {
        StackTraceElement[] stack = {
                new StackTraceElement("org.eclipse.core.internal.resources.Workspace", "build", "Workspace.java", 1),
                new StackTraceElement("org.eclipse.swt.widgets.Display", "asyncExec", "Display.java", 2),
                new StackTraceElement("com.anthropic.claudecode.eclipse.tools.BuildTool", "execute", "BuildTool.java", 3),
        };
        assertFalse(ToolDialogWatch.waitingOnUiThread(stack));
    }

    // --- whose dialog it is ----------------------------------------------------------------

    @Test
    void aDialogRaisedDuringACallIsOneThatWasNotOpenBeforeIt() {
        assertEquals(List.of("errors"),
                ToolDialogWatch.raisedBy(List.of("find", "errors"), List.of("find"), Set.of()));
    }

    @Test
    void aProgressDialogIsNotRaisedByTheCall() {
        assertEquals(List.of("errors"),
                ToolDialogWatch.raisedBy(List.of("progress", "errors"), List.of(), Set.of("progress")));
    }

    @Test
    void withNothingSeenBeforeTheCallEveryDialogThatAsksCounts() {
        assertEquals(List.of("a", "b"), ToolDialogWatch.raisedBy(List.of("a", "b"), null, Set.of()));
    }

    @Test
    void aThreadHeldBehindADialogIsThereOnBothChecks() {
        assertTrue(ToolDialogWatch.heldOnBoth(Set.of(7L, 9L), Set.of(9L, 12L)));
    }

    @Test
    void aThreadOnlyPassingThroughSyncExecIsNotHeld() {
        // A dialog the user opened: other threads' requests are run and let go.
        assertFalse(ToolDialogWatch.heldOnBoth(Set.of(7L), Set.of(8L)));
        assertFalse(ToolDialogWatch.heldOnBoth(Set.of(7L), Set.of()));
    }

    @Test
    void ofTwoDialogsTheOneSeenLaterIsInFront() {
        assertTrue(Dialogs.laterThan(5, 3));
        assertFalse(Dialogs.laterThan(3, 5));
    }

    @Test
    void dialogsSeenTogetherOrNeverSeenAreNotOrdered() {
        assertFalse(Dialogs.laterThan(3, 3));
        assertFalse(Dialogs.laterThan(null, 3));
        assertFalse(Dialogs.laterThan(3, null));
    }

    // --- prompts about trust are the user's ------------------------------------------------

    @Test
    void theInstallersTrustPromptIsToldByItsClass() {
        // p2's dialog, titled just "Trust"; the same goes for TrustAuthorityDialog.
        assertTrue(Dialogs.asksAboutTrust("", "", "TrustCertificateDialog SelectionDialog TrayDialog Dialog Window "));
        assertTrue(Dialogs.asksAboutTrust("", "", "TrustAuthorityDialog SelectionDialog TrayDialog Dialog Window "));
    }

    @Test
    void aTrustPromptThatIsAPlainMessageBoxIsToldByItsTitle() {
        assertTrue(Dialogs.asksAboutTrust("Always Trust Everything Confirmation", "Are you certain?", "MessageDialog "));
        assertTrue(Dialogs.asksAboutTrust("Host Key Change", "The host key for example.org has changed.", "MessageDialog "));
        assertTrue(Dialogs.asksAboutTrust("Certificate", "", "X509CertificateViewDialog TitleAreaDialog "));
    }

    @Test
    void aTrustPromptWithAPlainTitleIsToldByItsText() {
        assertTrue(Dialogs.asksAboutTrust("Warning",
                "The authenticity of host 'example.org' can't be established.", "MessageDialog "));
        assertTrue(Dialogs.asksAboutTrust("Security Warning",
                "Warning: Installing unsigned software for which the authenticity cannot be established.", null));
        assertTrue(Dialogs.asksAboutTrust("Question", "Do you trust these signers?", null));
    }

    @Test
    void aWizardAboutCertificatesIsToldByTheWizardNotTheDialog() {
        assertTrue(Dialogs.asksAboutTrust("Import", "", "WizardDialog TitleAreaDialog CertificateImportFileSelectPage CertificateImportWizard"));
    }

    @Test
    void ordinaryQuestionsAreNotAboutTrust() {
        assertFalse(Dialogs.asksAboutTrust("Errors in Workspace",
                "Errors exist in required project(s). Proceed with launch?", "MessageDialogWithToggle MessageDialog "));
        assertFalse(Dialogs.asksAboutTrust("Save Resource", "'A.java' has been modified. Save changes?", "MessageDialog "));
        assertFalse(Dialogs.asksAboutTrust("Select Java Application", "", "ElementListSelectionDialog "));
        assertFalse(Dialogs.asksAboutTrust("Rename", "unsigned int count;", "RefactoringWizardDialog "));
        assertFalse(Dialogs.asksAboutTrust(null, null, null));
    }

    // --- calls left running behind a dialog ------------------------------------------------

    @AfterEach
    void forgetParkedCalls() {
        ToolDialogWatch.forgetParked();
    }

    private static Future<McpToolResult> finished(String key) {
        JsonObject value = new JsonObject();
        value.addProperty(key, true);
        return CompletableFuture.completedFuture(McpToolResult.success(value));
    }

    private static JsonArray dialogs(String... ids) {
        JsonArray array = new JsonArray();
        for (String id : ids) {
            JsonObject dialog = new JsonObject();
            dialog.addProperty("id", id);
            array.add(dialog);
        }
        return array;
    }

    @Test
    void aParkedResultComesBackWithThePressOfItsOwnDialog() {
        String token = ToolDialogWatch.park("runAs", finished("launched"), Set.of("swt-1"));

        JsonArray handed = ToolDialogWatch.takeFinished("swt-1", null, 0);

        assertEquals(1, handed.size());
        JsonObject one = handed.get(0).getAsJsonObject();
        assertEquals("runAs", one.get("tool").getAsString());
        assertEquals(token, one.get("call").getAsString());
        assertTrue(one.getAsJsonObject("result").getAsJsonObject("value").get("launched").getAsBoolean());
    }

    @Test
    void aPressOfAnotherDialogDoesNotBringItBack() {
        ToolDialogWatch.park("runAs", finished("launched"), Set.of("swt-1"));

        assertEquals(0, ToolDialogWatch.takeFinished("swt-2", null, 0).size());
        assertEquals(1, ToolDialogWatch.takeFinished("swt-1", null, 0).size());
    }

    @Test
    void aParkedResultComesBackToWhoeverHoldsItsToken() {
        // The user answered the dialog by hand: there is no press to carry the result.
        String token = ToolDialogWatch.park("build", finished("built"), Set.of("swt-1"));

        assertEquals(0, ToolDialogWatch.takeFinished(null, "call-00000000", 0).size());
        assertEquals(1, ToolDialogWatch.takeFinished(null, token, 0).size());
    }

    @Test
    void askedWithNeitherADialogNorATokenNothingIsHandedOut() {
        // Another conversation listing the dialogs must not walk off with this one's result.
        String token = ToolDialogWatch.park("getSource", finished("source"), Set.of("swt-1"));

        assertEquals(0, ToolDialogWatch.takeFinished(null, null, 0).size());
        assertEquals(1, ToolDialogWatch.takeFinished(null, token, 0).size());
    }

    @Test
    void aResultIsHandedOutOnce() {
        ToolDialogWatch.park("runAs", finished("launched"), Set.of("swt-1"));

        assertEquals(1, ToolDialogWatch.takeFinished("swt-1", null, 0).size());
        assertEquals(0, ToolDialogWatch.takeFinished("swt-1", null, 0).size());
    }

    @Test
    void aCallStillRunningIsNamedAndNotAnsweredFor() {
        ToolDialogWatch.park("runTests", new CompletableFuture<>(), Set.of("swt-1"));

        assertEquals(0, ToolDialogWatch.takeFinished("swt-1", null, 0).size());
        assertEquals("runTests", ToolDialogWatch.stillParked().get(0).getAsString());
    }

    @Test
    void aCallGoesOnWaitingBehindTheDialogItsOwnDialogOpened() {
        // "Next >" in a wizard: the call is no nearer done, and the page is a new dialog.
        CompletableFuture<McpToolResult> call = new CompletableFuture<>();
        ToolDialogWatch.park("runAs", call, Set.of("swt-1"));

        ToolDialogWatch.follow("swt-1", Set.of("swt-2"));
        JsonObject value = new JsonObject();
        value.addProperty("launched", true);
        call.complete(McpToolResult.success(value));

        assertEquals(1, ToolDialogWatch.takeFinished("swt-2", null, 0).size());
    }

    @Test
    void aFinishedCallDoesNotTakeOnNewDialogs() {
        ToolDialogWatch.park("runAs", finished("launched"), Set.of("swt-1"));

        ToolDialogWatch.follow("swt-1", Set.of("swt-2"));

        assertEquals(0, ToolDialogWatch.takeFinished("swt-2", null, 0).size());
    }

    @Test
    void aFinishedCallNobodyCollectedIsStillNamed() {
        ToolDialogWatch.park("build", finished("built"), Set.of("swt-1"));

        assertEquals(0, ToolDialogWatch.stillParked().size());
        assertEquals("build", ToolDialogWatch.finishedUncollected().get(0).getAsString());
    }

    @Test
    void onlyACallStillRunningIsBehindItsDialog() {
        ToolDialogWatch.park("runAs", new CompletableFuture<>(), Set.of("swt-1"));
        ToolDialogWatch.park("build", finished("built"), Set.of("swt-2"));

        assertTrue(ToolDialogWatch.hasCallBehind("swt-1"));
        assertFalse(ToolDialogWatch.hasCallBehind("swt-2"));
        assertFalse(ToolDialogWatch.hasCallBehind("swt-3"));
        assertFalse(ToolDialogWatch.hasCallBehind(null));
    }

    @Test
    void theIdsOfAReportAreTheDialogsACallIsParkedBehind() {
        assertEquals(Set.of("swt-1", "os-7-aa"), ToolDialogWatch.idsOf(dialogs("swt-1", "os-7-aa")));
    }
}
