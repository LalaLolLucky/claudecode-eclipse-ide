package com.anthropic.claudecode.eclipse.tools.jdt;

import java.time.LocalTime;
import java.time.format.DateTimeFormatter;

import org.eclipse.debug.core.DebugException;
import org.eclipse.jdt.debug.core.IJavaDebugTarget;
import org.eclipse.jdt.debug.core.IJavaHotCodeReplaceListener;
import org.eclipse.jdt.debug.core.JDIDebugModel;

import com.anthropic.claudecode.eclipse.tools.HotCodeReplaceStatus;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;

/**
 * Records JDT's hot code replace outcomes into {@link HotCodeReplaceStatus}, so the
 * {@code debug} tool can say whether an edit made while a Java program is paused actually
 * reached the running program — or failed (a changed method signature, an added field) and
 * needs a restart. Without it, a failed replace is silent and the program keeps running the
 * old code.
 *
 * <p>Needs {@code org.eclipse.jdt.debug}; installed only when it is present.
 */
public final class HotCodeReplaceMonitor implements IJavaHotCodeReplaceListener {

	private static boolean installed;

	private HotCodeReplaceMonitor() {}

	/** Registers the listener once, however many times the tool registry is built. */
	public static synchronized void install() {
		if (installed) return;
		JDIDebugModel.addHotCodeReplaceListener(new HotCodeReplaceMonitor());
		installed = true;
	}

	private static String now() {
		return LocalTime.now().format(DateTimeFormatter.ofPattern("HH:mm:ss"));
	}

	@Override
	public void hotCodeReplaceSucceeded(IJavaDebugTarget target) {
		HotCodeReplaceStatus.record(target, "succeeded at " + now());
		ClaudeCodeView.debug("[debug] hot code replace succeeded");
	}

	@Override
	public void hotCodeReplaceFailed(IJavaDebugTarget target, DebugException exception) {
		String reason = exception != null && exception.getMessage() != null ? exception.getMessage() : "unknown reason";
		HotCodeReplaceStatus.record(target, "FAILED at " + now() + ": " + reason
				+ " — the program is still running the old code; restart it to pick up the change.");
		ClaudeCodeView.debug("[debug] hot code replace failed: " + reason);
	}

	@Override
	public void obsoleteMethods(IJavaDebugTarget target) {
		HotCodeReplaceStatus.record(target, "applied at " + now() + ", but frames on the stack are "
				+ "still running the old version of changed methods (obsolete); drop to frame or "
				+ "step out of them to run the new code.");
	}
}
