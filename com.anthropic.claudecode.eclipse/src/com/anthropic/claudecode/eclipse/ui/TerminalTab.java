package com.anthropic.claudecode.eclipse.ui;

/**
 * One tab of the Claude Terminal view, as the claudeCodeEclipse tool sees it.
 *
 * @param id      names the tab to the tool ("term1", "term2", …), in the order tabs were opened
 * @param title   what the tab strip shows
 * @param active  whether it is the tab in front
 * @param started whether Claude has started in it
 * @param ended   whether Claude has ended in it
 */
public record TerminalTab(String id, String title, boolean active, boolean started, boolean ended) {
}
