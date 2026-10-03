package com.anthropic.claudecode.eclipse.tools;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;

import java.lang.reflect.Proxy;
import java.net.URI;

import org.eclipse.core.resources.IProject;
import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.Path;
import org.junit.jupiter.api.Test;

import com.google.gson.JsonObject;

/**
 * Tests for {@link GetWorkspaceFoldersTool#describe(IProject)}, the per-project half of the tool,
 * against {@link Proxy} projects so no workspace is needed.
 */
class GetWorkspaceFoldersToolTest {

    private static IProject project(boolean open, IPath location, URI locationUri) {
        return (IProject) Proxy.newProxyInstance(IProject.class.getClassLoader(),
                new Class<?>[] { IProject.class }, (self, method, args) -> switch (method.getName()) {
                    case "isOpen" -> open;
                    case "getName" -> "demo";
                    case "getLocation" -> location;
                    case "getLocationURI" -> locationUri;
                    default -> null;
                });
    }

    @Test
    void aProjectOnDiskKeepsItsUriNameAndPath() {
        IPath location = Path.fromOSString(System.getProperty("java.io.tmpdir")).append("demo");
        URI uri = location.toFile().toURI();

        JsonObject folder = GetWorkspaceFoldersTool.describe(project(true, location, uri));

        assertEquals(uri.toString(), folder.get("uri").getAsString());
        assertEquals("demo", folder.get("name").getAsString());
        assertEquals(location.toOSString(), folder.get("path").getAsString());
    }

    @Test
    void aProjectWithNoDiskLocationIsListedByUriWithoutAPath() {
        URI uri = URI.create("semanticfs:/E10_900_user_de");

        JsonObject folder = GetWorkspaceFoldersTool.describe(project(true, null, uri));

        assertEquals(uri.toString(), folder.get("uri").getAsString());
        assertEquals("demo", folder.get("name").getAsString());
        assertFalse(folder.has("path"));
    }

    @Test
    void aClosedProjectIsLeftOut() {
        IPath location = Path.fromOSString(System.getProperty("java.io.tmpdir")).append("demo");
        assertNull(GetWorkspaceFoldersTool.describe(project(false, location, location.toFile().toURI())));
    }

    @Test
    void aProjectWithNoLocationAtAllIsLeftOut() {
        assertNull(GetWorkspaceFoldersTool.describe(project(true, null, null)));
    }
}
