package com.anthropic.claudecode.eclipse.tools.jdt;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;

import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Proxy;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicReference;

import org.eclipse.debug.core.ILaunch;
import org.eclipse.debug.core.model.IDebugTarget;
import org.eclipse.debug.core.model.IProcess;
import org.eclipse.jdt.core.IClasspathEntry;
import org.eclipse.jdt.core.IJavaElement;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.IPackageFragment;
import org.eclipse.jdt.core.IPackageFragmentRoot;
import org.eclipse.jdt.core.IType;
import org.eclipse.jdt.core.JavaModelException;
import org.junit.jupiter.api.Test;

/**
 * Tests for {@link RunTestsTool#testKindFor}, which picks the JUnit runner for a launch. JDT's own
 * chooser needs a workspace, so it is passed in; the project is a {@link Proxy} fake.
 */
class RunTestsToolTest {

    private static final String JUNIT4 = "org.eclipse.jdt.junit.loader.junit4";
    private static final String JUNIT5 = "org.eclipse.jdt.junit.loader.junit5";
    private static final String JUNIT6 = "org.eclipse.jdt.junit.loader.junit6";

    private static <T> T fake(Class<T> type, InvocationHandler handler) {
        return type.cast(Proxy.newProxyInstance(type.getClassLoader(), new Class<?>[] { type }, handler));
    }

    /** A launch with {@code processes} processes and no debug target. */
    private static ILaunch launch(int processes) {
        return fake(ILaunch.class, (self, method, args) -> switch (method.getName()) {
            case "getProcesses" -> new IProcess[processes];
            case "getDebugTargets" -> new IDebugTarget[0];
            default -> null;
        });
    }

    @Test
    void aLaunchRefusedBeforeItStartedIsNotWaitedFor() {
        // What "Errors exist… Proceed?" answered with Cancel leaves: nothing running, and
        // the launch taken back out of the launch manager.
        assertTrue(RunTestsTool.neverStarted(launch(0), l -> false));
    }

    @Test
    void aLaunchThatIsRunningIsWaitedFor() {
        assertFalse(RunTestsTool.neverStarted(launch(1), l -> true));
    }

    @Test
    void aLaunchStillRegisteredIsWaitedForEvenBeforeItsProcessShows() {
        assertFalse(RunTestsTool.neverStarted(launch(0), l -> true));
    }

    @Test
    void noLaunchAtAllNeverStarted() {
        assertTrue(RunTestsTool.neverStarted(null, l -> true));
    }

    /** A project whose {@code findType} is answered by {@code findType}. */
    private static IJavaProject project(InvocationHandler findType) {
        return fake(IJavaProject.class, (self, method, args) ->
                method.getName().equals("findType") ? findType.invoke(self, method, args) : null);
    }

    private static IJavaProject projectWithJupiter(boolean present) {
        IType jupiterTest = fake(IType.class, (self, method, args) -> null);
        return project((self, method, args) ->
                present && "org.junit.jupiter.api.Test".equals(args[0]) ? jupiterTest : null);
    }

    /** A project that fails the test if anything looks a type up in it. */
    private static IJavaProject projectThatIsNotSearched() {
        return project((self, method, args) -> {
            throw new AssertionError("JDT answered, so the project is not searched");
        });
    }

    private static IJavaElement elementIn(IJavaProject project) {
        return fake(IType.class, (self, method, args) ->
                method.getName().equals("getJavaProject") ? project : null);
    }

    /** A fake answering the named methods with fixed values; anything else gives false, 0 or null. */
    private static <T> T stub(Class<T> type, Map<String, Object> answers) {
        return fake(type, (self, method, args) -> {
            if (answers.containsKey(method.getName())) return answers.get(method.getName());
            Class<?> returns = method.getReturnType();
            if (returns == boolean.class) return false;
            if (returns == int.class) return 0;
            return null;
        });
    }

    private static IPackageFragment packageWithJava(boolean hasJava) {
        return stub(IPackageFragment.class, Map.of("exists", true, "containsJavaResources", hasJava));
    }

    /** A source or library folder holding {@code packages}; any other name gives a package that does not exist. */
    private static IPackageFragmentRoot folder(int kind, boolean testCode, Map<String, IPackageFragment> packages) {
        IClasspathEntry entry = stub(IClasspathEntry.class, Map.of("isTest", testCode));
        IPackageFragment missing = stub(IPackageFragment.class, Map.of());
        return fake(IPackageFragmentRoot.class, (self, method, args) -> switch (method.getName()) {
            case "getKind" -> kind;
            case "getRawClasspathEntry" -> entry;
            case "getPackageFragment" -> packages.getOrDefault(args[0], missing);
            default -> null;
        });
    }

    private static IPackageFragmentRoot sourceFolder(boolean testCode, Map<String, IPackageFragment> packages) {
        return folder(IPackageFragmentRoot.K_SOURCE, testCode, packages);
    }

    private static IJavaProject projectWith(IPackageFragmentRoot... folders) {
        return stub(IJavaProject.class, Map.of("getPackageFragmentRoots", folders));
    }

    // ---- findPackage: a package name as the target ----

    @Test
    void aPackageIsFoundInASourceFolder() {
        IPackageFragment demo = packageWithJava(true);
        IJavaProject project = projectWith(sourceFolder(false, Map.of("demo", demo)));
        assertSame(demo, RunTestsTool.findPackage(List.of(project), "demo"));
    }

    @Test
    void aPackageInBothSourceAndTestFoldersRunsFromTheTestFolder() {
        IPackageFragment inSrc = packageWithJava(true);
        IPackageFragment inTest = packageWithJava(true);
        IJavaProject project = projectWith(
                sourceFolder(false, Map.of("demo", inSrc)),
                sourceFolder(true, Map.of("demo", inTest)));
        assertSame(inTest, RunTestsTool.findPackage(List.of(project), "demo"));
    }

    @Test
    void aPackageWithNoJavaFilesIsPassedOver() {
        IPackageFragment empty = packageWithJava(false);
        IPackageFragment real = packageWithJava(true);
        IJavaProject project = projectWith(
                sourceFolder(true, Map.of("demo", empty)),
                sourceFolder(false, Map.of("demo", real)));
        assertSame(real, RunTestsTool.findPackage(List.of(project), "demo"));
    }

    @Test
    void librariesAreNotSearched() {
        IJavaProject project = projectWith(
                folder(IPackageFragmentRoot.K_BINARY, false, Map.of("demo", packageWithJava(true))));
        assertNull(RunTestsTool.findPackage(List.of(project), "demo"));
    }

    @Test
    void theFirstProjectWithThePackageWins() {
        IPackageFragment first = packageWithJava(true);
        IJavaProject without = projectWith(sourceFolder(false, Map.of("other", packageWithJava(true))));
        IJavaProject plain = projectWith(sourceFolder(false, Map.of("demo", first)));
        IJavaProject withTestFolder = projectWith(sourceFolder(true, Map.of("demo", packageWithJava(true))));
        assertSame(first, RunTestsTool.findPackage(List.of(without, plain, withTestFolder), "demo"));
    }

    @Test
    void anUnknownPackageGivesNone() {
        IJavaProject project = projectWith(sourceFolder(false, Map.of("demo", packageWithJava(true))));
        assertNull(RunTestsTool.findPackage(List.of(project), "nope"));
        assertNull(RunTestsTool.findPackage(List.of(), "demo"));
    }

    @Test
    void aProjectThatCannotBeReadIsSkipped() {
        IPackageFragment demo = packageWithJava(true);
        IJavaProject broken = fake(IJavaProject.class, (self, method, args) -> {
            throw new JavaModelException(new IllegalStateException("closed"), 0);
        });
        IJavaProject good = projectWith(sourceFolder(false, Map.of("demo", demo)));
        assertSame(demo, RunTestsTool.findPackage(List.of(broken, good), "demo"));
    }

    // ---- JDT's choice comes first ----

    @Test
    void jdtsChoiceIsUsedAsIs() {
        IJavaElement element = elementIn(projectThatIsNotSearched());
        assertEquals(JUNIT6, RunTestsTool.testKindFor(element, e -> JUNIT6));
    }

    @Test
    void aRunnerThisToolHasNeverHeardOfIsStillUsed() {
        IJavaElement element = elementIn(projectThatIsNotSearched());
        assertEquals("org.eclipse.jdt.junit.loader.junit7",
                RunTestsTool.testKindFor(element, e -> "org.eclipse.jdt.junit.loader.junit7"));
    }

    @Test
    void jdtIsAskedAboutTheTargetItself() {
        IJavaElement element = elementIn(projectThatIsNotSearched());
        AtomicReference<IJavaElement> asked = new AtomicReference<>();
        RunTestsTool.testKindFor(element, e -> {
            asked.set(e);
            return JUNIT5;
        });
        assertSame(element, asked.get());
    }

    // ---- the guess, for when JDT cannot be asked ----

    @Test
    void whenJdtsChooserFailsAJupiterProjectRunsOnJUnit5() {
        IJavaElement element = elementIn(projectWithJupiter(true));
        assertEquals(JUNIT5, RunTestsTool.testKindFor(element, e -> {
            throw new IllegalStateException("chooser failed");
        }));
    }

    @Test
    void whenJdtsChooserIsGoneAJupiterProjectRunsOnJUnit5() {
        IJavaElement element = elementIn(projectWithJupiter(true));
        assertEquals(JUNIT5, RunTestsTool.testKindFor(element, e -> {
            throw new NoClassDefFoundError("org/eclipse/jdt/internal/junit/launcher/TestKindRegistry");
        }));
    }

    @Test
    void whenJdtsChooserFailsAProjectWithoutJupiterRunsOnJUnit4() {
        IJavaElement element = elementIn(projectWithJupiter(false));
        assertEquals(JUNIT4, RunTestsTool.testKindFor(element, e -> {
            throw new IllegalStateException("chooser failed");
        }));
    }

    @Test
    void whenJdtGivesNoAnswerTheGuessIsUsed() {
        assertEquals(JUNIT5, RunTestsTool.testKindFor(elementIn(projectWithJupiter(true)), e -> null));
        assertEquals(JUNIT4, RunTestsTool.testKindFor(elementIn(projectWithJupiter(false)), e -> " "));
    }

    @Test
    void aProjectThatCannotBeSearchedRunsOnJUnit4() {
        IJavaElement element = elementIn(project((self, method, args) -> {
            throw new JavaModelException(new IllegalStateException("closed"), 0);
        }));
        assertEquals(JUNIT4, RunTestsTool.testKindFor(element, e -> null));
    }
}
