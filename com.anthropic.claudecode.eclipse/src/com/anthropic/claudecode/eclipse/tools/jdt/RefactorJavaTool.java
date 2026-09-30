package com.anthropic.claudecode.eclipse.tools.jdt;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IFolder;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.jdt.core.ICompilationUnit;
import org.eclipse.jdt.core.IField;
import org.eclipse.jdt.core.IJavaElement;
import org.eclipse.jdt.core.IMethod;
import org.eclipse.jdt.core.IPackageFragment;
import org.eclipse.jdt.core.IPackageFragmentRoot;
import org.eclipse.jdt.core.IType;
import org.eclipse.jdt.core.refactoring.IJavaRefactorings;
import org.eclipse.jdt.core.refactoring.descriptors.MoveDescriptor;
import org.eclipse.jdt.core.refactoring.descriptors.RenameJavaElementDescriptor;
import org.eclipse.ltk.core.refactoring.Refactoring;
import org.eclipse.ltk.core.refactoring.RefactoringContribution;
import org.eclipse.ltk.core.refactoring.RefactoringCore;
import org.eclipse.ltk.core.refactoring.RefactoringDescriptor;
import org.eclipse.ltk.core.refactoring.RefactoringStatus;
import org.eclipse.ui.IEditorPart;
import org.eclipse.ui.IWorkbenchPage;
import org.eclipse.ui.IWorkbenchWindow;
import org.eclipse.ui.PlatformUI;

import com.anthropic.claudecode.eclipse.editor.UiHelper;
import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.tools.RefactoringRunner;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Renames a Java type, method or field, or moves a type to another package, through JDT's
 * refactorings — the tool form of Refactor › Rename and Refactor › Move. Every reference in
 * the workspace is updated semantically (overrides, imports, qualified uses), where a text
 * replace would miss some and hit unrelated same-named symbols; installed plug-ins' own
 * participants join in, and the whole change is one entry under Edit › Undo.
 *
 * <p>Adapted from JDT Bridge's {@code RefactoringHandler.handleRename/handleMove}, with two
 * differences: the change runs through {@link RefactoringRunner}, which records it with the
 * refactoring undo manager, and an ERROR-level condition refuses the refactoring unless
 * {@code force} is set (JDT Bridge only stops on FATAL) — the Rename dialog stops and asks at
 * that point too.
 *
 * <p>Needs {@code org.eclipse.jdt.core} and {@code org.eclipse.jdt.core.manipulation}
 * (where the descriptors live); registered only when both are installed.
 */
public class RefactorJavaTool implements McpTool {

	@Override
	public String toolName() {
		return "refactorJava";
	}

	@Override
	public String description() {
		return "Rename a Java type, method or field, or move a type to another package, with JDT's "
				+ "refactorings (Refactor > Rename / Move): every reference in the workspace is "
				+ "updated semantically — overriding methods, imports, qualified uses — and the whole "
				+ "change can be undone with Edit > Undo. Prefer this to editing references by hand. "
				+ "'fqn' names the element: 'com.example.Foo', 'com.example.Foo.bar(String,int)' or "
				+ "'com.example.Foo.FIELD'. Refused when an editor has unsaved changes, or when JDT "
				+ "reports an error (pass force=true to proceed past errors, not fatal ones).";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject schema = new JsonObject();
		schema.addProperty("type", "object");

		JsonObject props = new JsonObject();

		JsonObject action = new JsonObject();
		action.addProperty("type", "string");
		JsonArray actions = new JsonArray();
		actions.add("rename");
		actions.add("move");
		action.add("enum", actions);
		action.addProperty("description", "rename (type, method or field; needs 'newName') or move (a type; needs 'targetPackage').");
		props.add("action", action);

		props.add("fqn", prop("string", "The element: a type, Type.method(ParamType,...) or Type.FIELD."));
		props.add("newName", prop("string", "For rename: the new simple name."));
		props.add("targetPackage", prop("string", "For move: the destination package, in the same "
				+ "source folder as the type; created if it does not exist."));
		props.add("force", prop("boolean", "Proceed even when JDT reports ERROR-level problems "
				+ "(default false). Fatal problems always refuse."));

		schema.add("properties", props);
		JsonArray required = new JsonArray();
		required.add("action");
		required.add("fqn");
		schema.add("required", required);
		return schema;
	}

	private static JsonObject prop(String type, String description) {
		JsonObject p = new JsonObject();
		p.addProperty("type", type);
		p.addProperty("description", description);
		return p;
	}

	@Override
	public McpToolResult execute(JsonObject params) {
		try {
			String action = str(params, "action");
			String fqn = str(params, "fqn");
			if (action == null || fqn == null) return McpToolResult.error("Missing required parameters: action and fqn.");
			boolean force = params.has("force") && !params.get("force").isJsonNull() && params.get("force").getAsBoolean();

			IJavaElement element = JdtUtils.resolveElement(fqn);
			if (element == null) {
				return McpToolResult.error("Could not resolve element: " + fqn
						+ ". Use a fully qualified type name, Type.method(ParamType,...) or Type.FIELD.");
			}
			if (element instanceof org.eclipse.jdt.core.IMember member && member.isBinary()) {
				return McpToolResult.error(fqn + " is in a library, not in the workspace's source, so it cannot be refactored.");
			}

			String dirty = UiHelper.syncCall(RefactorJavaTool::dirtyEditors);
			if (dirty != null && !dirty.isEmpty()) {
				return McpToolResult.error("Save or revert these editors first — a refactoring rewrites "
						+ "files that may be open: " + dirty);
			}

			RefactoringDescriptor descriptor;
			String summary;
			if ("rename".equals(action)) {
				String newName = str(params, "newName");
				if (newName == null) return McpToolResult.error("action='rename' needs 'newName'.");
				String id = element instanceof IType ? IJavaRefactorings.RENAME_TYPE
						: element instanceof IMethod ? IJavaRefactorings.RENAME_METHOD
						: element instanceof IField ? IJavaRefactorings.RENAME_FIELD : null;
				if (id == null) return McpToolResult.error("Only types, methods and fields can be renamed here.");
				RefactoringDescriptor d = newDescriptor(id);
				if (!(d instanceof RenameJavaElementDescriptor rename)) {
					return McpToolResult.error("JDT's rename refactoring is not available in this Eclipse.");
				}
				rename.setJavaElement(element);
				rename.setNewName(newName);
				rename.setUpdateReferences(true);
				descriptor = rename;
				summary = "renamed " + JdtUtils.getFqn(element) + " to " + newName;
			} else if ("move".equals(action)) {
				if (!(element instanceof IType type)) return McpToolResult.error("Only types can be moved here.");
				String targetPackage = str(params, "targetPackage");
				if (targetPackage == null) return McpToolResult.error("action='move' needs 'targetPackage'.");
				ICompilationUnit cu = type.getCompilationUnit();
				if (cu == null) return McpToolResult.error("A binary type cannot be moved.");
				if (type.getDeclaringType() != null) {
					return McpToolResult.error("A nested type cannot be moved on its own; move its top-level type.");
				}
				IPackageFragmentRoot sourceRoot = (IPackageFragmentRoot) cu.getAncestor(IJavaElement.PACKAGE_FRAGMENT_ROOT);
				IPackageFragment destination = sourceRoot.getPackageFragment(targetPackage);
				if (!destination.exists()) {
					destination = sourceRoot.createPackageFragment(targetPackage, true, new NullProgressMonitor());
				}
				RefactoringDescriptor d = newDescriptor(IJavaRefactorings.MOVE);
				if (!(d instanceof MoveDescriptor move)) {
					return McpToolResult.error("JDT's move refactoring is not available in this Eclipse.");
				}
				move.setMoveResources(new IFile[0], new IFolder[0], new ICompilationUnit[] { cu });
				move.setDestination(destination);
				move.setUpdateReferences(true);
				descriptor = move;
				summary = "moved " + type.getFullyQualifiedName() + " to package " + targetPackage;
			} else {
				return McpToolResult.error("Unknown action: " + action + ". Use rename or move.");
			}

			RefactoringStatus createStatus = new RefactoringStatus();
			Refactoring refactoring = descriptor.createRefactoring(createStatus);
			if (refactoring == null || createStatus.hasFatalError()) {
				return McpToolResult.error(RefactoringRunner.describe("The refactoring could not be created", createStatus));
			}

			RefactoringRunner.Outcome outcome = RefactoringRunner.run(refactoring, force);
			if (!outcome.performed()) {
				boolean fatal = outcome.conditions() != null && outcome.conditions().hasFatalError();
				return McpToolResult.error(RefactoringRunner.describe("Refused — nothing was changed"
						+ (fatal || force ? "" : " (pass force=true to proceed past errors)"),
						outcome.conditions(), outcome.validation()));
			}

			JsonObject out = new JsonObject();
			out.addProperty("result", summary);
			JsonArray problems = RefactoringRunner.messages(outcome.conditions());
			if (!problems.isEmpty()) out.add("problems", problems);
			ClaudeCodeView.debug("[refactorJava] " + summary);
			return McpToolResult.success(out);
		} catch (Exception | LinkageError e) {
			ClaudeCodeView.debug("[refactorJava] failed: " + e);
			return McpToolResult.error("refactorJava failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
		}
	}

	private static RefactoringDescriptor newDescriptor(String id) {
		RefactoringContribution contribution = RefactoringCore.getRefactoringContribution(id);
		return contribution == null ? null : contribution.createDescriptor();
	}

	/** On the UI thread: titles of editors with unsaved changes, in any window. */
	private static String dirtyEditors() {
		StringBuilder sb = new StringBuilder();
		for (IWorkbenchWindow window : PlatformUI.getWorkbench().getWorkbenchWindows()) {
			for (IWorkbenchPage page : window.getPages()) {
				for (IEditorPart editor : page.getDirtyEditors()) {
					if (sb.length() > 0) sb.append(", ");
					sb.append(editor.getTitle());
				}
			}
		}
		return sb.toString();
	}

	private static String str(JsonObject params, String key) {
		if (!params.has(key) || params.get(key).isJsonNull()) return null;
		String v = params.get(key).getAsString().trim();
		return v.isEmpty() ? null : v;
	}
}
