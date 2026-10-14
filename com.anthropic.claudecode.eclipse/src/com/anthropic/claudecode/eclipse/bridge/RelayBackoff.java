package com.anthropic.claudecode.eclipse.bridge;

/**
 * How long the relay watchdog waits before its next attempt, counted in ticks.
 * <p>
 * Each failed start waits one tick longer than the last, up to {@code maxTicks}. A
 * relay that comes up only clears that streak once a later tick still finds it
 * running: one that is gone before then counts as one more failed start. Without
 * that, a relay that died right after every handshake was rebuilt on every tick,
 * forever, since each rebuild looked like a success.
 * <p>
 * Not thread-safe; the Activator calls it under its own lock.
 */
public final class RelayBackoff {

    private final int maxTicks;
    private int failureStreak;
    private int ticksToSkip;
    /** Up, but no tick has seen it running yet. */
    private boolean settling;

    public RelayBackoff(int maxTicks) {
        this.maxTicks = maxTicks;
    }

    /** A deliberate start (launch, Restart Server, the view's toggle): no wait at all. */
    public void reset() {
        failureStreak = 0;
        ticksToSkip = 0;
        settling = false;
    }

    /** True while this tick is being skipped for backoff, using it up. */
    public boolean consumeTick() {
        if (ticksToSkip > 0) {
            ticksToSkip--;
            return true;
        }
        return false;
    }

    /** A start that failed: the next attempt waits one tick longer than the last. */
    public void failed() {
        if (failureStreak < maxTicks) {
            failureStreak++;
        }
        ticksToSkip = failureStreak;
        settling = false;
    }

    /** The relay came up and both halves connected. Settled at the next tick that finds it running. */
    public void connected() {
        settling = true;
    }

    /** A tick found the relay running. */
    public void stillRunning() {
        if (settling) {
            settling = false;
            failureStreak = 0;
            ticksToSkip = 0;
        }
    }

    /**
     * Called when the relay is found down, before it is replaced. True when it went down
     * before it settled: that counts as a failed start, and the caller waits the backoff
     * out instead of replacing it now.
     */
    public boolean droppedBeforeSettling() {
        if (!settling) {
            return false;
        }
        failed();
        return true;
    }

    /** The relay was taken down on purpose (server gone, shutdown): not a failure. */
    public void stopped() {
        settling = false;
    }
}
