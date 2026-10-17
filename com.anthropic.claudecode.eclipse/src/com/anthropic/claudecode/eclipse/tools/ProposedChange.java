package com.anthropic.claudecode.eclipse.tools;

import java.nio.file.Files;
import java.nio.file.Path;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

/**
 * What a file-writing tool call would do to its file: the text before and the text after. Shown
 * as a diff while the permission card is up, where the reader may change the "after" side; the
 * changed text is turned back into the tool's own input by {@link #editedInput}.
 */
public final class ProposedChange {

    /** The file the call writes. */
    public final String filePath;
    /** Its text now; empty for a file that does not exist yet. */
    public final String original;
    /** Its text once the call has run. */
    public final String proposed;

    private ProposedChange(String filePath, String original, String proposed) {
        this.filePath = filePath;
        this.original = original;
        this.proposed = proposed;
    }

    /**
     * The change a file-editing call proposes, or null when the call is not one (Read, Bash,
     * search, ...) or its result cannot be worked out.
     */
    public static ProposedChange of(String toolName, JsonElement inputEl) {
        if (inputEl == null || !inputEl.isJsonObject()) return null;
        JsonObject in = inputEl.getAsJsonObject();
        String fp = str(in, "file_path");
        if (fp == null) return null;
        String tn = toolName == null ? "" : toolName.toLowerCase();
        try {
            Path p = Path.of(fp);
            String orig = Files.exists(p) ? Files.readString(p) : "";
            if (tn.contains("write")) {
                String content = str(in, "content");
                if (content != null) return new ProposedChange(fp, orig, content);
            }
            if (in.has("edits") && in.get("edits").isJsonArray()) {
                String cur = orig;
                for (JsonElement ee : in.getAsJsonArray("edits")) {
                    if (!ee.isJsonObject()) continue;
                    JsonObject e = ee.getAsJsonObject();
                    cur = applyEdit(cur, str(e, "old_string"), str(e, "new_string"), bool(e, "replace_all"));
                }
                return new ProposedChange(fp, orig, cur);
            }
            String oldS = str(in, "old_string"), newS = str(in, "new_string");
            if (oldS != null && newS != null) {
                return new ProposedChange(fp, orig, applyEdit(orig, oldS, newS, bool(in, "replace_all")));
            }
        } catch (Exception ignored) {}
        return null;
    }

    /**
     * The call's input with the "after" text as the reader left it, or null when this kind of
     * call cannot take an edited result (a multi-part edit) or nothing changed.
     *
     * <p>A Write takes the text whole. An Edit takes the smallest span of lines that holds the
     * change and is found once in the file, as its {@code old_string} and {@code new_string}.
     */
    public JsonObject editedInput(String toolName, JsonObject input, String edited) {
        if (edited == null || edited.equals(proposed)) return null;
        String tn = toolName == null ? "" : toolName.toLowerCase();
        JsonObject out = input.deepCopy();
        if (tn.contains("write") && input.has("content")) {
            out.addProperty("content", edited);
            return out;
        }
        if (input.has("old_string") && input.has("new_string") && !input.has("edits")) {
            String[] span = minimalEdit(original, edited);
            if (span == null) return null;
            out.addProperty("old_string", span[0]);
            out.addProperty("new_string", span[1]);
            out.addProperty("replace_all", false);
            return out;
        }
        return null;
    }

    /**
     * {@code {old, new}}: replacing the first by the second, once, in {@code orig} gives
     * {@code edited}. Whole lines, widened by neighbouring lines until the old text occurs
     * once. Null when the two are the same.
     */
    static String[] minimalEdit(String orig, String edited) {
        if (orig.equals(edited)) return null;
        if (orig.isEmpty()) return new String[] { "", edited };
        int max = Math.min(orig.length(), edited.length());
        int p = 0;
        while (p < max && orig.charAt(p) == edited.charAt(p)) p++;
        int s = 0;
        while (s < max - p && orig.charAt(orig.length() - 1 - s) == edited.charAt(edited.length() - 1 - s)) s++;

        int a = orig.lastIndexOf('\n', p - 1) + 1;
        int b = orig.length() - s;
        int eb = edited.length() - s;
        int lineEnd = orig.indexOf('\n', b);
        int extra = (lineEnd < 0 ? orig.length() : lineEnd) - b;
        b += extra;
        eb += extra;

        while (true) {
            String old = orig.substring(a, b);
            if (!old.isEmpty() && orig.indexOf(old, orig.indexOf(old) + 1) < 0) break;
            if (a == 0 && b == orig.length()) break;
            if (a > 0) a = orig.lastIndexOf('\n', a - 2) + 1;
            if (b < orig.length()) {
                int next = orig.indexOf('\n', b + 1);
                int add = (next < 0 ? orig.length() : next) - b;
                b += add;
                eb += add;
            }
        }
        return new String[] { orig.substring(a, b), edited.substring(a, eb) };
    }

    private static String applyEdit(String content, String oldS, String newS, boolean all) {
        if (oldS == null || newS == null || oldS.isEmpty()) return content;
        if (all) return content.replace(oldS, newS);
        int i = content.indexOf(oldS);
        return i < 0 ? content : content.substring(0, i) + newS + content.substring(i + oldS.length());
    }

    private static String str(JsonObject o, String k) {
        return (o.has(k) && o.get(k).isJsonPrimitive()) ? o.get(k).getAsString() : null;
    }

    private static boolean bool(JsonObject o, String k) {
        return o.has(k) && o.get(k).isJsonPrimitive()
                && o.get(k).getAsJsonPrimitive().isBoolean() && o.get(k).getAsBoolean();
    }
}
