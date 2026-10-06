package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Set;

import org.junit.jupiter.api.Test;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Tests for how the native core's dialog listing is merged with Eclipse's own, which needs
 * neither a display nor the native library.
 */
class NativeDialogsTest {

    private static JsonObject dialog(String id, String title, boolean external) {
        JsonObject j = new JsonObject();
        j.addProperty("id", id);
        j.addProperty("title", title);
        j.addProperty("external", external);
        return j;
    }

    private static JsonArray of(JsonObject... dialogs) {
        JsonArray array = new JsonArray();
        for (JsonObject dialog : dialogs) array.add(dialog);
        return array;
    }

    @Test
    void anIdFromTheNativeCoreIsToldApartFromAnSwtOne() {
        assertTrue(NativeDialogs.isNativeId("os-4242-1a2b"));
        assertFalse(NativeDialogs.isNativeId("swt-1a2b"));
        assertFalse(NativeDialogs.isNativeId(null));
    }

    @Test
    void aDialogOfThisEclipseSeenByBothSidesIsListedOnce() {
        JsonArray swt = of(dialog("swt-1", "Save Resource", false));
        JsonArray natives = of(dialog("os-7-aa", "Save Resource", false), dialog("os-7-bb", "Open File", false));

        JsonArray kept = NativeDialogs.withoutThoseIn(swt, natives, true);

        assertEquals(1, kept.size());
        assertEquals("os-7-bb", kept.get(0).getAsJsonObject().get("id").getAsString());
    }

    @Test
    void aWindowOfThisProcessIsListedByTheCoreUnderItsHandle() {
        // As the core listed a real one: process 33252, window handle 0x160fe0.
        assertEquals("os-33252-313630666530", NativeDialogs.idOfOwnWindow(33252, 0x160fe0L));
    }

    @Test
    void thisEclipsesOwnShellIsNotListedASecondTimeByItsHandle() {
        // Windows: an SWT dialog shell is a Win32 dialog box too, so the core reports it.
        JsonArray swt = of(dialog("swt-1", "Search", false));
        JsonArray natives = of(dialog("os-7-aa", "Search", false), dialog("os-7-bb", "Search", false));

        JsonArray kept = NativeDialogs.withoutThoseIn(swt, natives, false, Set.of("os-7-aa"));

        // The file chooser the Search dialog opened may carry its title; only the shell goes.
        assertEquals(1, kept.size());
        assertEquals("os-7-bb", kept.get(0).getAsJsonObject().get("id").getAsString());
    }

    @Test
    void withNoShellsOfItsOwnNothingIsDroppedWhereTheCoreSeesOnlyNativeDialogs() {
        JsonArray swt = of(dialog("swt-1", "Search", false));
        JsonArray natives = of(dialog("os-7-aa", "Search", false));

        assertEquals(1, NativeDialogs.withoutThoseIn(swt, natives, false, Set.of()).size());
    }

    @Test
    void anotherEclipsesDialogIsNeverTakenForOneOfThisEclipsesShells() {
        JsonArray natives = of(dialog("os-9-aa", "Search", true));

        assertEquals(1, NativeDialogs.withoutThoseIn(new JsonArray(), natives, false, Set.of("os-9-aa")).size());
    }

    @Test
    void anotherEclipsesDialogIsKeptEvenUnderTheSameTitle() {
        JsonArray swt = of(dialog("swt-1", "Save Resource", false));
        JsonArray natives = of(dialog("os-9-cc", "Save Resource", true));

        assertEquals(1, NativeDialogs.withoutThoseIn(swt, natives, true).size());
    }

    @Test
    void withNoSwtDialogsEveryNativeOneIsKept() {
        JsonArray natives = of(dialog("os-7-aa", "Information", false), dialog("os-9-cc", "Wizard", true));

        assertEquals(2, NativeDialogs.withoutThoseIn(new JsonArray(), natives, true).size());
    }

    @Test
    void anotherEclipsesTrustPromptIsMarkedAsTheUsers() {
        JsonObject trust = dialog("os-9-aa", "Trust", true);
        JsonObject hostKey = dialog("os-9-bb", "Warning", true);
        hostKey.addProperty("text", "The authenticity of host 'example.org' can't be established.");
        JsonObject save = dialog("os-9-cc", "Save Resource", true);
        save.addProperty("text", "'A.java' has been modified. Save changes?");

        NativeDialogs.markForUserOnly(of(trust, hostKey, save));

        assertTrue(trust.has("forUserOnly"));
        assertTrue(hostKey.has("forUserOnly"));
        assertFalse(save.has("forUserOnly"));
    }

    @Test
    void whereTheNativeSideNeverListsSwtShellsANativeDialogOfTheSameTitleIsKept() {
        // A "Save As" dialog of Eclipse's that has opened the system's "Save As" chooser.
        JsonArray swt = of(dialog("swt-1", "Save As", false));
        JsonArray natives = of(dialog("os-7-aa", "Save As", false));

        JsonArray kept = NativeDialogs.withoutThoseIn(swt, natives, false);

        assertEquals(1, kept.size());
        assertEquals("os-7-aa", kept.get(0).getAsJsonObject().get("id").getAsString());
    }
}
