package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;

import org.junit.jupiter.api.Test;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Tests for the parts of {@link EclipseCustomizePerspectiveTool} that need no workbench: the
 * hidden-items string, the shortcut lists, labels and the arguments.
 */
class EclipseCustomizePerspectiveToolTest {

    private static JsonObject json(String s) {
        return JsonParser.parseString(s).getAsJsonObject();
    }

    @Test
    void hiddenIdsReadsOnlyItsOwnPrefix() {
        String hidden = "persp.hideMenuSC:org.eclipse.ui.file.save,persp.hideToolbarSC:org.eclipse.debug.ui.launchActionSet,"
                + "persp.hideMenuSC:navigate,persp.hideActionSetSC:org.eclipse.search.searchActionSet,";
        assertEquals(List.of("org.eclipse.ui.file.save", "navigate"),
                EclipseCustomizePerspectiveTool.hiddenIds(hidden, EclipseCustomizePerspectiveTool.HIDDEN_MENU));
        assertEquals(List.of("org.eclipse.debug.ui.launchActionSet"),
                EclipseCustomizePerspectiveTool.hiddenIds(hidden, EclipseCustomizePerspectiveTool.HIDDEN_TOOLBAR));
        assertEquals(List.of("org.eclipse.search.searchActionSet"),
                EclipseCustomizePerspectiveTool.hiddenIds(hidden, EclipseCustomizePerspectiveTool.HIDDEN_ACTION_SET));
    }

    @Test
    void hiddenIdsOfNothing() {
        assertTrue(EclipseCustomizePerspectiveTool.hiddenIds(null, EclipseCustomizePerspectiveTool.HIDDEN_MENU).isEmpty());
        assertTrue(EclipseCustomizePerspectiveTool.hiddenIds("", EclipseCustomizePerspectiveTool.HIDDEN_MENU).isEmpty());
        // A bare prefix names nothing; a repeated entry is one id.
        assertEquals(List.of("a"), EclipseCustomizePerspectiveTool.hiddenIds(
                "persp.hideMenuSC:,persp.hideMenuSC:a,persp.hideMenuSC:a,", EclipseCustomizePerspectiveTool.HIDDEN_MENU));
    }

    @Test
    void showingAShortcutKeepsTheOthers() {
        // setNewShortcuts replaces the whole list under its tag, so one more must carry the rest.
        assertEquals(List.of("a", "b", "c"),
                EclipseCustomizePerspectiveTool.mergedShortcuts(new String[] {"a", "b"}, List.of("c", "a"), true));
    }

    @Test
    void hidingAShortcutRemovesOnlyIt() {
        assertEquals(List.of("a", "c"),
                EclipseCustomizePerspectiveTool.mergedShortcuts(new String[] {"a", "b", "c"}, List.of("b", "x"), false));
        assertEquals(List.of("x"), EclipseCustomizePerspectiveTool.mergedShortcuts(null, List.of("x"), true));
    }

    @Test
    void labelsLoseMnemonicAndAccelerator() {
        assertEquals("Open File...", EclipseCustomizePerspectiveTool.cleanLabel("&Open File...\tCtrl+O"));
        assertEquals("Save & Close", EclipseCustomizePerspectiveTool.cleanLabel("Save && &Close"));
        assertNull(EclipseCustomizePerspectiveTool.cleanLabel("&"));
        assertNull(EclipseCustomizePerspectiveTool.cleanLabel(null));
    }

    @Test
    void kindsAndTheirStorage() {
        assertEquals("actionSet", EclipseCustomizePerspectiveTool.kindOf(" ActionSet "));
        assertNull(EclipseCustomizePerspectiveTool.kindOf("shortcut"));
        assertNull(EclipseCustomizePerspectiveTool.kindOf(null));
        assertEquals("persp.newWizSC:", EclipseCustomizePerspectiveTool.shortcutTag("newWizard"));
        assertEquals("persp.perspSC:", EclipseCustomizePerspectiveTool.shortcutTag("perspective"));
        assertEquals("persp.viewSC:", EclipseCustomizePerspectiveTool.shortcutTag("view"));
        assertNull(EclipseCustomizePerspectiveTool.shortcutTag("menu"));
        assertEquals("persp.hideMenuSC:", EclipseCustomizePerspectiveTool.hiddenPrefix("menu"));
        assertEquals("persp.hideToolbarSC:", EclipseCustomizePerspectiveTool.hiddenPrefix("toolbar"));
        assertEquals("persp.hideActionSetSC:", EclipseCustomizePerspectiveTool.hiddenPrefix("actionSet"));
        assertNull(EclipseCustomizePerspectiveTool.hiddenPrefix("view"));
    }

    @Test
    void idsAcceptAnArrayOrOneString() {
        assertEquals(List.of("a", "b"), EclipseCustomizePerspectiveTool.idsOf(json("{\"ids\":[\" a \",\"b\",\"a\",\"\"]}")));
        assertEquals(List.of("a"), EclipseCustomizePerspectiveTool.idsOf(json("{\"ids\":\"a\"}")));
        assertTrue(EclipseCustomizePerspectiveTool.idsOf(json("{}")).isEmpty());
        assertTrue(EclipseCustomizePerspectiveTool.idsOf(json("{\"ids\":null}")).isEmpty());
    }

    @Test
    void argumentsAreCheckedBeforeTheWorkbench() {
        List<String> none = List.of();
        List<String> one = List.of("a");
        assertNull(EclipseCustomizePerspectiveTool.argumentError(null, null, none), "no arguments means get");
        assertNull(EclipseCustomizePerspectiveTool.argumentError("GET", null, none));
        assertNull(EclipseCustomizePerspectiveTool.argumentError("open", null, none));
        assertNull(EclipseCustomizePerspectiveTool.argumentError("list", "menu", none));
        assertNull(EclipseCustomizePerspectiveTool.argumentError("hide", "Toolbar", one));
        assertTrue(EclipseCustomizePerspectiveTool.argumentError("list", null, none).contains("'kind' is required"));
        assertTrue(EclipseCustomizePerspectiveTool.argumentError("hide", "shortcut", one).contains("'kind' is required"));
        assertTrue(EclipseCustomizePerspectiveTool.argumentError("show", "view", none).contains("'ids' is required"));
        assertTrue(EclipseCustomizePerspectiveTool.argumentError("reset", null, none).startsWith("Unknown action: reset"));
        // The whole main toolbar takes no ids; its parts do.
        assertNull(EclipseCustomizePerspectiveTool.argumentError("hide", "mainToolbar", none));
        assertNull(EclipseCustomizePerspectiveTool.argumentError("show", "MainToolbar", none));
        assertTrue(EclipseCustomizePerspectiveTool.argumentError("hide", "toolbar", none).contains("'ids' is required"));
    }
}
