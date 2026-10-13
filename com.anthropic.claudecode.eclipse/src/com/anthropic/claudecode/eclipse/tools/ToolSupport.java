package com.anthropic.claudecode.eclipse.tools;

import java.io.File;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IProject;
import org.eclipse.core.resources.IWorkspaceRoot;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.Path;
import org.eclipse.core.runtime.Platform;
import org.eclipse.ui.IEditorPart;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.part.FileEditorInput;
import org.osgi.framework.Bundle;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * What the workspace tools share: describing a parameter in a schema, reading one from a
 * call, finding the projects or the file a call names, and calling into a plug-in that may
 * not be installed.
 */
public final class ToolSupport {

    private ToolSupport() {}

    /** A call that cannot be carried out, with the reason to answer it with. */
    public static final class Refusal extends Exception {
        private static final long serialVersionUID = 1L;

        public Refusal(String reason) {
            super(reason);
        }
    }

    public static JsonObject prop(String type, String description) {
        JsonObject p = new JsonObject();
        p.addProperty("type", type);
        p.addProperty("description", description);
        return p;
    }

    public static JsonObject strings(String description) {
        JsonObject p = prop("array", description);
        JsonObject items = new JsonObject();
        items.addProperty("type", "string");
        p.add("items", items);
        return p;
    }

    public static JsonObject choice(String description, List<String> values) {
        JsonObject p = prop("string", description);
        JsonArray allowed = new JsonArray();
        for (String v : values) allowed.add(v);
        p.add("enum", allowed);
        return p;
    }

    public static JsonObject schema(JsonObject props, String... required) {
        JsonObject schema = new JsonObject();
        schema.addProperty("type", "object");
        schema.add("properties", props);
        if (required.length > 0) {
            JsonArray names = new JsonArray();
            for (String name : required) names.add(name);
            schema.add("required", names);
        }
        return schema;
    }

    /** A string parameter, trimmed; null when it is absent or empty. */
    public static String str(JsonObject params, String key) {
        if (!params.has(key) || !params.get(key).isJsonPrimitive()) return null;
        String v = params.get(key).getAsString().trim();
        return v.isEmpty() ? null : v;
    }

    public static boolean bool(JsonObject params, String key, boolean whenAbsent) {
        if (!params.has(key) || !params.get(key).isJsonPrimitive()) return whenAbsent;
        return params.get(key).getAsBoolean();
    }

    /** A whole-number parameter, held between {@code min} and {@code max}. */
    public static int integer(JsonObject params, String key, int whenAbsent, int min, int max) {
        if (!params.has(key) || !params.get(key).isJsonPrimitive()) return whenAbsent;
        try {
            return Math.max(min, Math.min(max, params.get(key).getAsInt()));
        } catch (RuntimeException e) {
            return whenAbsent;
        }
    }

    /** A list-of-strings parameter; empty when it is absent. */
    public static List<String> list(JsonObject params, String key) {
        List<String> out = new ArrayList<>();
        if (params.has(key) && params.get(key).isJsonArray()) {
            for (var e : params.getAsJsonArray(key)) {
                if (e.isJsonPrimitive() && !e.getAsString().isBlank()) out.add(e.getAsString().trim());
            }
        }
        return out;
    }

    /**
     * The projects named in {@code projects}, each of which has to exist and be open; every
     * open project when none is named. The same rule {@link BuildTool} follows.
     */
    public static List<IProject> projects(JsonObject params, String verb) throws Refusal {
        IWorkspaceRoot root = ResourcesPlugin.getWorkspace().getRoot();
        List<String> named = list(params, "projects");
        List<IProject> targets = new ArrayList<>();
        if (named.isEmpty()) {
            for (IProject p : root.getProjects()) {
                if (p.isAccessible()) targets.add(p);
            }
            return targets;
        }
        List<String> missing = new ArrayList<>();
        for (String name : named) {
            IProject p = root.getProject(name);
            if (!p.exists()) { missing.add(name + " (no such project)"); continue; }
            if (!p.isAccessible()) { missing.add(name + " (closed)"); continue; }
            targets.add(p);
        }
        if (!missing.isEmpty()) throw new Refusal("Cannot " + verb + ": " + String.join(", ", missing));
        return targets;
    }

    /**
     * The workspace file at an absolute path: the one that exists, or, when none does, the
     * handle of where it would be (a deleted file still has a local history). Null when the
     * path is not inside a workspace project.
     */
    public static IFile workspaceFile(String path) {
        IWorkspaceRoot root = ResourcesPlugin.getWorkspace().getRoot();
        for (IFile f : root.findFilesForLocationURI(new File(path).toURI())) {
            if (f.exists()) return f;
        }
        return root.getFileForLocation(new Path(path));
    }

    /** Whether the file is open in an editor with changes that are not saved. */
    public static boolean hasUnsavedChanges(IFile file) {
        return Boolean.TRUE.equals(UiHelper.syncCall(() -> {
            IEditorPart editor = editorOf(file);
            return editor != null && editor.isDirty();
        }));
    }

    /** The editor the file is open in, or null. UI thread only. */
    static IEditorPart editorOf(IFile file) {
        IWorkbenchPage page = UiHelper.getActivePage();
        return page == null ? null : page.findEditor(new FileEditorInput(file));
    }

    /** Whether a plug-in is installed. */
    public static boolean installed(String bundleId) {
        Bundle bundle = Platform.getBundle(bundleId);
        return bundle != null && bundle.getState() != Bundle.UNINSTALLED;
    }

    /**
     * A class of a plug-in this one does not compile against, so that the plug-in being
     * absent leaves a tool unregistered instead of this bundle unresolved.
     */
    public static Class<?> classOf(String bundleId, String className) throws ClassNotFoundException {
        Bundle bundle = Platform.getBundle(bundleId);
        if (bundle == null) throw new ClassNotFoundException(bundleId + " is not installed");
        return bundle.loadClass(className);
    }

    /**
     * Calls the public method {@code name} that {@code type} declares or inherits and that
     * takes these arguments, on {@code target} (null for a static one). The method is looked
     * up on {@code type}, an API type, because the object's own class is often not public.
     */
    public static Object call(Class<?> type, Object target, String name, Object... args) throws Exception {
        for (Method m : type.getMethods()) {
            if (!m.getName().equals(name) || m.getParameterCount() != args.length) continue;
            Class<?>[] wanted = m.getParameterTypes();
            boolean fits = true;
            for (int i = 0; i < args.length && fits; i++) {
                fits = args[i] == null ? !wanted[i].isPrimitive() : boxed(wanted[i]).isInstance(args[i]);
            }
            if (!fits) continue;
            try {
                return m.invoke(target, args);
            } catch (InvocationTargetException e) {
                if (e.getCause() instanceof Exception cause) throw cause;
                throw e;
            }
        }
        throw new NoSuchMethodException(type.getName() + "." + name);
    }

    private static Class<?> boxed(Class<?> type) {
        if (type == boolean.class) return Boolean.class;
        if (type == int.class) return Integer.class;
        if (type == long.class) return Long.class;
        return type;
    }
}
