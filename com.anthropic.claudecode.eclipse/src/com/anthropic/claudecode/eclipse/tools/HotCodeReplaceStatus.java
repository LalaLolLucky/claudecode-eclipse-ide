package com.anthropic.claudecode.eclipse.tools;

import java.util.Map;
import java.util.WeakHashMap;

import org.eclipse.debug.core.model.IDebugTarget;

/**
 * The last hot code replace outcome per debug target, for {@link DebugTool} to report.
 *
 * <p>Hot code replace is Java-only and reported by {@code org.eclipse.jdt.debug}, an optional
 * dependency. This holder uses platform debug types only, so {@link DebugTool} can read it
 * without ever loading a JDT class; the JDT listener that fills it lives in
 * {@code tools.jdt} and is installed only when {@code org.eclipse.jdt.debug} is present.
 * Weak keys, so terminated targets are dropped with the launch.
 */
public final class HotCodeReplaceStatus {

    private static final Map<IDebugTarget, String> LAST = new WeakHashMap<>();

    private HotCodeReplaceStatus() {}

    public static synchronized void record(IDebugTarget target, String outcome) {
        LAST.put(target, outcome);
    }

    public static synchronized String get(IDebugTarget target) {
        return LAST.get(target);
    }
}
