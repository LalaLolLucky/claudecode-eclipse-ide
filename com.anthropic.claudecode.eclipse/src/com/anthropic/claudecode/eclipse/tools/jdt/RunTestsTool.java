package com.anthropic.claudecode.eclipse.tools.jdt;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.function.Function;
import java.util.function.Predicate;

import org.eclipse.core.resources.IProject;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.debug.core.DebugPlugin;
import org.eclipse.debug.core.ILaunch;
import org.eclipse.debug.core.ILaunchConfiguration;
import org.eclipse.debug.core.ILaunchConfigurationType;
import org.eclipse.debug.core.ILaunchConfigurationWorkingCopy;
import org.eclipse.debug.core.ILaunchManager;
import org.eclipse.jdt.core.IClasspathEntry;
import org.eclipse.jdt.core.IJavaElement;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.IMethod;
import org.eclipse.jdt.core.IPackageFragment;
import org.eclipse.jdt.core.IPackageFragmentRoot;
import org.eclipse.jdt.core.IType;
import org.eclipse.jdt.core.JavaModelException;
import org.eclipse.jdt.internal.junit.launcher.TestKindRegistry;
import org.eclipse.jdt.junit.JUnitCore;
import org.eclipse.jdt.junit.TestRunListener;
import org.eclipse.jdt.junit.model.ITestCaseElement;
import org.eclipse.jdt.junit.model.ITestElement;
import org.eclipse.jdt.junit.model.ITestElementContainer;
import org.eclipse.jdt.junit.model.ITestRunSession;
import org.eclipse.jdt.launching.IJavaLaunchConfigurationConstants;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Runs JUnit tests and returns results.
 * Adapted from JDT Bridge's TestHandler.
 */
public class RunTestsTool implements McpTool {

	private static final String JUNIT_LAUNCH_TYPE = "org.eclipse.jdt.junit.launchconfig";
	private static final String JUNIT4_KIND = "org.eclipse.jdt.junit.loader.junit4";
	private static final String JUNIT5_KIND = "org.eclipse.jdt.junit.loader.junit5";

	@Override
	public String toolName() {
		return "runTests";
	}

	@Override
	public String description() {
		return "Run JUnit tests for a class, method, package, or project. "
				+ "Returns pass/fail counts and failure details. "
				+ "Input: fully qualified name of test class/method/package, or project name.";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject schema = new JsonObject();
		schema.addProperty("type", "object");

		JsonObject props = new JsonObject();

		JsonObject target = new JsonObject();
		target.addProperty("type", "string");
		target.addProperty("description", "Test target: FQN of class/method/package, or project name");
		props.add("target", target);

		JsonObject timeout = new JsonObject();
		timeout.addProperty("type", "integer");
		timeout.addProperty("description", "Timeout in seconds (default: 300)");
		props.add("timeout", timeout);

		schema.add("properties", props);

		JsonArray required = new JsonArray();
		required.add("target");
		schema.add("required", required);

		return schema;
	}

	@Override
	public McpToolResult execute(JsonObject params) {
		try {
			String target = params.has("target") ? params.get("target").getAsString() : null;
			if (target == null || target.isBlank()) {
				return McpToolResult.error("Missing required parameter: target");
			}

			int timeoutSecs = params.has("timeout") ? params.get("timeout").getAsInt() : 300;

			// Resolve target to determine test scope
			TestTarget testTarget = resolveTarget(target);
			if (testTarget == null) {
				return McpToolResult.error("Could not resolve test target: " + target);
			}

			// Create launch configuration
			ILaunchConfiguration config = createLaunchConfig(testTarget);
			if (config == null) {
				return McpToolResult.error("Could not create launch configuration for: " + target);
			}

			// Set up result collector
			TestResultCollector collector = new TestResultCollector();
			JUnitCore.addTestRunListener(collector);

			try {
				// Launch tests
				ILaunch launch = config.launch(ILaunchManager.RUN_MODE, new NullProgressMonitor());

				// Refused before it began — "Errors exist… Proceed?" answered with Cancel, say.
				// No test will ever report, so there is nothing to wait the timeout out for.
				if (neverStarted(launch, DebugPlugin.getDefault().getLaunchManager()::isRegistered)) {
					JsonObject result = new JsonObject();
					result.addProperty("target", target);
					result.addProperty("completed", false);
					result.addProperty("started", false);
					result.addProperty("totalCount", 0);
					result.addProperty("passCount", 0);
					result.addProperty("failCount", 0);
					result.addProperty("errorCount", 0);
					result.addProperty("skipCount", 0);
					result.addProperty("note", "The launch did not start: it was cancelled or refused "
							+ "before any test ran.");
					return McpToolResult.success(result);
				}

				// Wait for completion
				boolean completed = collector.awaitCompletion(timeoutSecs, TimeUnit.SECONDS);

				// Wait a bit more for launch to fully terminate
				int waitCount = 0;
				while (!launch.isTerminated() && waitCount < 30) {
					Thread.sleep(100);
					waitCount++;
				}

				JsonObject result = new JsonObject();
				result.addProperty("target", target);
				result.addProperty("completed", completed);
				result.addProperty("totalCount", collector.getTotalCount());
				result.addProperty("passCount", collector.getPassCount());
				result.addProperty("failCount", collector.getFailCount());
				result.addProperty("errorCount", collector.getErrorCount());
				result.addProperty("skipCount", collector.getSkipCount());

				if (!collector.getFailures().isEmpty()) {
					JsonArray failures = new JsonArray();
					for (TestFailure f : collector.getFailures()) {
						JsonObject fj = new JsonObject();
						fj.addProperty("testName", f.testName);
						fj.addProperty("className", f.className);
						fj.addProperty("status", f.status);
						if (f.trace != null) {
							fj.addProperty("trace", f.trace);
						}
						failures.add(fj);
					}
					result.add("failures", failures);
				}

				return McpToolResult.success(result);
			} finally {
				JUnitCore.removeTestRunListener(collector);
			}
		} catch (Exception e) {
			return McpToolResult.error("Failed to run tests: " + e.getMessage());
		}
	}

	/**
	 * Whether a launch came back without ever starting. Eclipse takes a launch that is
	 * refused before it begins back out of the launch manager, and it has nothing running;
	 * one that did start is registered, with its process, by the time launching returns.
	 * Both are asked, so a launch whose process merely is not there yet is still waited for.
	 */
	static boolean neverStarted(ILaunch launch, Predicate<ILaunch> registered) {
		if (launch == null) return true;
		return launch.getProcesses().length == 0 && launch.getDebugTargets().length == 0
				&& !registered.test(launch);
	}

	private TestTarget resolveTarget(String target) {
		// Try as method: com.example.FooTest.testBar()
		IJavaElement element = JdtUtils.resolveElement(target);
		if (element instanceof IMethod method) {
			IType type = method.getDeclaringType();
			return new TestTarget(type, type.getFullyQualifiedName(), method.getElementName());
		}

		// Try as type
		if (element instanceof IType type) {
			return new TestTarget(type, type.getFullyQualifiedName(), null);
		}

		// Try as project name, before packages: a plug-in project is usually named
		// after its root package, and that name has always meant the whole project
		List<IJavaProject> projects = JdtUtils.getJavaProjects();
		for (IJavaProject jp : projects) {
			if (jp.getElementName().equals(target)) {
				return new TestTarget(jp, null, null);
			}
		}

		// Try as package
		IPackageFragment pkg = findPackage(projects, target);
		if (pkg != null) {
			return new TestTarget(pkg, null, null);
		}

		return null;
	}

	/**
	 * The package named {@code name} to run tests from, or null. A package can sit in
	 * several source folders (src and test) and several projects, and a launch takes one of
	 * them: the first project that has it with Java files, and there a folder marked as test
	 * code before any other.
	 */
	static IPackageFragment findPackage(List<IJavaProject> projects, String name) {
		for (IJavaProject project : projects) {
			try {
				IPackageFragment first = null;
				for (IPackageFragmentRoot root : project.getPackageFragmentRoots()) {
					if (root.getKind() != IPackageFragmentRoot.K_SOURCE) {
						continue;
					}
					IPackageFragment pkg = root.getPackageFragment(name);
					if (pkg == null || !pkg.exists() || !pkg.containsJavaResources()) {
						continue;
					}
					IClasspathEntry entry = root.getRawClasspathEntry();
					if (entry != null && entry.isTest()) {
						return pkg;
					}
					if (first == null) {
						first = pkg;
					}
				}
				if (first != null) {
					return first;
				}
			} catch (JavaModelException e) {
				// Continue to next project
			}
		}
		return null;
	}

	private ILaunchConfiguration createLaunchConfig(TestTarget target) throws Exception {
		ILaunchManager manager = DebugPlugin.getDefault().getLaunchManager();
		ILaunchConfigurationType type = manager.getLaunchConfigurationType(JUNIT_LAUNCH_TYPE);
		if (type == null) {
			return null;
		}

		String configName = "Claude Test Run - " + System.currentTimeMillis();
		ILaunchConfigurationWorkingCopy config = type.newInstance(null, configName);

		config.setAttribute(IJavaLaunchConfigurationConstants.ATTR_PROJECT_NAME, target.project.getName());

		String testKind = testKindFor(target.element, RunTestsTool::jdtTestKind);
		config.setAttribute("org.eclipse.jdt.junit.TEST_KIND", testKind);

		if (target.className != null) {
			config.setAttribute(IJavaLaunchConfigurationConstants.ATTR_MAIN_TYPE_NAME, target.className);
			if (target.methodName != null) {
				config.setAttribute("org.eclipse.jdt.junit.TESTNAME", target.methodName);
			}
		} else {
			// Run all tests in the package or project: JDT reads this back with
			// JavaCore.create(String), which takes an element handle, not a name
			config.setAttribute("org.eclipse.jdt.junit.CONTAINER", target.element.getHandleIdentifier());
		}

		return config;
	}

	/**
	 * The JUnit runner to launch {@code element} with. JDT picks it, the way Run As &gt; JUnit Test
	 * does, so it is whichever JUnit the project is on (3, 4, 5, 6, or one newer than this tool)
	 * and one this IDE has a runner for. JDT refuses a launch whose runner does not match the
	 * JUnit on the build path, so a choice made here can be turned down; its own cannot.
	 *
	 * <p>{@code jdtChoice} is a seam: JDT's chooser needs a workspace.
	 */
	static String testKindFor(IJavaElement element, Function<IJavaElement, String> jdtChoice) {
		try {
			String kind = jdtChoice.apply(element);
			if (kind != null && !kind.isBlank()) {
				return kind;
			}
		} catch (RuntimeException | LinkageError e) {
			// JDT's chooser is internal: one that is gone or fails leaves the guess below.
		}
		return guessTestKind(element.getJavaProject());
	}

	@SuppressWarnings("restriction") // org.eclipse.jdt.internal.junit.launcher — the chooser behind Run As > JUnit Test (no public JDT equivalent)
	private static String jdtTestKind(IJavaElement element) {
		return TestKindRegistry.getContainerTestKindId(element);
	}

	/** JUnit 5 when the Jupiter API is on the build path, else JUnit 4. */
	private static String guessTestKind(IJavaProject project) {
		try {
			if (project.findType("org.junit.jupiter.api.Test") != null) {
				return JUNIT5_KIND;
			}
		} catch (Exception e) {
			// Fall through
		}
		return JUNIT4_KIND;
	}

	private static class TestTarget {
		/** What to run: a type (also for a single method), a package, or a project. */
		final IJavaElement element;
		final IProject project;
		final String className;
		final String methodName;

		TestTarget(IJavaElement element, String className, String methodName) {
			this.element = element;
			this.project = element.getJavaProject().getProject();
			this.className = className;
			this.methodName = methodName;
		}
	}

	private static class TestFailure {
		final String testName;
		final String className;
		final String status;
		final String trace;

		TestFailure(String testName, String className, String status, String trace) {
			this.testName = testName;
			this.className = className;
			this.status = status;
			this.trace = trace;
		}
	}

	private static class TestResultCollector extends TestRunListener {
		private final CountDownLatch latch = new CountDownLatch(1);
		private final List<TestFailure> failures = new ArrayList<>();
		private int totalCount = 0;
		private int passCount = 0;
		private int failCount = 0;
		private int errorCount = 0;
		private int skipCount = 0;

		@Override
		public void sessionFinished(ITestRunSession session) {
			countResults(session.getChildren());
			totalCount = passCount + failCount + errorCount + skipCount;
			latch.countDown();
		}

		private void countResults(ITestElement[] elements) {
			for (ITestElement element : elements) {
				if (element instanceof ITestCaseElement tc) {
					// ITestElement.Result is a type-safe-enum class, not a Java enum,
					// so it can't be used in a switch — compare by identity instead.
					ITestElement.Result result = tc.getTestResult(false);
					if (result == ITestElement.Result.OK) {
						passCount++;
					} else if (result == ITestElement.Result.FAILURE) {
						failCount++;
						addFailure(tc, "FAILURE");
					} else if (result == ITestElement.Result.ERROR) {
						errorCount++;
						addFailure(tc, "ERROR");
					} else if (result == ITestElement.Result.IGNORED) {
						skipCount++;
					}
				} else if (element instanceof ITestElementContainer container) {
					countResults(container.getChildren());
				}
			}
		}

		private void addFailure(ITestCaseElement tc, String status) {
			String trace = null;
			try {
				var failureTrace = tc.getFailureTrace();
				if (failureTrace != null) {
					trace = failureTrace.getTrace();
				}
			} catch (Exception e) {
				// Ignore
			}
			failures.add(new TestFailure(
					tc.getTestMethodName(),
					tc.getTestClassName(),
					status,
					trace));
		}

		boolean awaitCompletion(long timeout, TimeUnit unit) throws InterruptedException {
			return latch.await(timeout, unit);
		}

		int getTotalCount() { return totalCount; }
		int getPassCount() { return passCount; }
		int getFailCount() { return failCount; }
		int getErrorCount() { return errorCount; }
		int getSkipCount() { return skipCount; }
		List<TestFailure> getFailures() { return failures; }
	}
}
