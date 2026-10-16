package com.anthropic.claudecode.eclipse.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayList;
import java.util.List;

import org.junit.jupiter.api.Test;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonParser;

/**
 * Tests for reading the core's account-models answer and curating it into the model
 * chooser's rows, on answers written out here so no network or login is needed.
 */
class ModelCatalogTest {

    /** The models the API listed for a Pro account on 2026-10-16, as the core passes them on. */
    private static final String LISTED = "{\"ok\":true,\"data\":["
            + "{\"id\":\"claude-haiku-5-5\",\"display_name\":\"Claude Haiku 5.5\"},"
            + "{\"id\":\"claude-sonnet-5-5\",\"display_name\":\"Claude Sonnet 5.5\"},"
            + "{\"id\":\"claude-opus-5-5\",\"display_name\":\"Claude Opus 5.5\"},"
            + "{\"id\":\"claude-fable-5-1\",\"display_name\":\"Claude Fable 5.1\"},"
            + "{\"id\":\"claude-opus-5\",\"display_name\":\"Claude Opus 5\"},"
            + "{\"id\":\"claude-fable-5\",\"display_name\":\"Claude Fable 5\"},"
            + "{\"id\":\"claude-opus-4-8\",\"display_name\":\"Claude Opus 4.8\"},"
            + "{\"id\":\"claude-opus-4-5-20251101\",\"display_name\":\"Claude Opus 4.5\"},"
            + "{\"id\":\"claude-sonnet-4-5-20250929\",\"display_name\":\"Claude Sonnet 4.5\"}]}";

    private static List<String> field(String json, String name) {
        List<String> out = new ArrayList<>();
        for (JsonElement e : JsonParser.parseString(json).getAsJsonArray())
            out.add(e.getAsJsonObject().get(name).getAsString());
        return out;
    }

    @Test
    void theCoresListIsRead() {
        ModelCatalog.Answer a = ModelCatalog.answerOf(LISTED);
        assertEquals(9, a.data().size());
        assertNull(a.state());
        assertNull(a.why());
    }

    @Test
    void noListSaysWhy() {
        ModelCatalog.Answer a = ModelCatalog.answerOf(
                "{\"ok\":false,\"state\":\"signed-out\",\"error\":\"no claude.ai login\"}");
        assertNull(a.data());
        assertEquals("signed-out", a.state());
        assertEquals("no claude.ai login", a.why());
    }

    @Test
    void anUnreadableAnswerIsNoListAndAnError() {
        for (String answer : new String[] { null, "", "not json", "[]", "{\"ok\":true}" }) {
            ModelCatalog.Answer a = ModelCatalog.answerOf(answer);
            assertNull(a.data(), String.valueOf(answer));
            assertEquals("error", a.state(), String.valueOf(answer));
        }
    }

    @Test
    void onlyAnErrorIsWorthAskingAgain() {
        assertTrue(ModelCatalog.worthRetrying("error"));
        for (String state : new String[] { "signed-out", "expired", "empty", null })
            assertFalse(ModelCatalog.worthRetrying(state), String.valueOf(state));
    }

    @Test
    void theNewestOfEachFamilyIsListedByItsVersionedName() {
        String rows = ModelCatalog.curate(ModelCatalog.answerOf(LISTED).data());
        assertEquals(List.of("", "fable", "opus", "sonnet", "sonnet[1m]", "haiku"), field(rows, "id"));
        assertEquals(List.of("Default (recommended)", "Fable 5.1", "Opus 5.5", "Sonnet 5.5",
                "Sonnet 5.5 (1M context)", "Haiku 5.5"), field(rows, "label"));
        assertEquals(List.of("", "claude-fable-5-1", "claude-opus-5-5", "claude-sonnet-5-5",
                "claude-sonnet-5-5", "claude-haiku-5-5"), field(rows, "fullId"));
    }

    @Test
    void aFamilyNotKnownHereIsListedByItsFullIdAfterTheKnownOnes() {
        JsonArray data = JsonParser.parseString("[{\"id\":\"claude-opus-5-5\",\"display_name\":\"Claude Opus 5.5\"},"
                + "{\"id\":\"claude-quill-1\",\"display_name\":\"Claude Quill 1\"}]").getAsJsonArray();
        String rows = ModelCatalog.curate(data);
        assertEquals(List.of("", "opus", "claude-quill-1"), field(rows, "id"));
        assertEquals("Quill 1", field(rows, "label").get(2));
    }

    @Test
    void aListThatNamesNoModelIsNone() {
        assertNull(ModelCatalog.curate(new JsonArray()));
        assertNull(ModelCatalog.curate(JsonParser.parseString("[{\"display_name\":\"no id\"}]").getAsJsonArray()));
    }

    @Test
    void theBuiltInListNamesNoVersion() {
        assertEquals(List.of("Default (recommended)", "Fable", "Opus", "Sonnet", "Sonnet (1M context)", "Haiku"),
                field(ModelCatalog.fallbackJson(), "label"));
    }
}
