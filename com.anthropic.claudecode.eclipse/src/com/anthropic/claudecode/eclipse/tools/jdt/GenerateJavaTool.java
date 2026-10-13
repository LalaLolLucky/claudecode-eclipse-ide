package com.anthropic.claudecode.eclipse.tools.jdt;

import java.io.ByteArrayInputStream;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

import org.eclipse.core.resources.IFile;
import org.eclipse.core.resources.IResource;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.jdt.core.ICompilationUnit;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.IMethod;
import org.eclipse.jdt.core.ISourceRange;
import org.eclipse.jdt.core.IType;
import org.eclipse.jdt.core.NamingConventions;
import org.eclipse.jdt.core.dom.AST;
import org.eclipse.jdt.core.dom.ASTNode;
import org.eclipse.jdt.core.dom.ASTParser;
import org.eclipse.jdt.core.dom.AbstractTypeDeclaration;
import org.eclipse.jdt.core.dom.CompilationUnit;
import org.eclipse.jdt.core.dom.FieldDeclaration;
import org.eclipse.jdt.core.dom.IMethodBinding;
import org.eclipse.jdt.core.dom.ITypeBinding;
import org.eclipse.jdt.core.dom.MethodDeclaration;
import org.eclipse.jdt.core.dom.Modifier;
import org.eclipse.jdt.core.dom.NodeFinder;
import org.eclipse.jdt.core.dom.PrimitiveType;
import org.eclipse.jdt.core.dom.SingleVariableDeclaration;
import org.eclipse.jdt.core.dom.Type;
import org.eclipse.jdt.core.dom.TypeDeclaration;
import org.eclipse.jdt.core.dom.VariableDeclarationFragment;
import org.eclipse.jdt.core.dom.rewrite.ASTRewrite;
import org.eclipse.jdt.core.dom.rewrite.ImportRewrite;
import org.eclipse.jdt.core.dom.rewrite.ListRewrite;
import org.eclipse.jface.text.Document;
import org.eclipse.text.edits.TextEdit;

import com.anthropic.claudecode.eclipse.mcp.McpTool;
import com.anthropic.claudecode.eclipse.mcp.McpToolResult;
import com.anthropic.claudecode.eclipse.ui.ClaudeCodeView;
import com.anthropic.claudecode.eclipse.tools.ToolSupport;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * The Source menu's generators for a Java class: getters and setters, a constructor from
 * fields, and stubs that override or implement inherited methods.
 *
 * <p>Built on JDT's public API only. The members are written as text and put into the type
 * through an {@link ASTRewrite}, which indents them to where they land; the types an
 * overriding stub names go through an {@link ImportRewrite}, which adds the imports they
 * need. JDT's own generator operations are internal and are not used. Like
 * {@link OrganizeImportsTool}, it works on the file as saved and writes the file.
 */
public class GenerateJavaTool implements McpTool {

	private static final List<String> ACTIONS = List.of("gettersSetters", "constructor", "overrideMethods");
	private static final List<String> KINDS = List.of("both", "getters", "setters");

	@Override
	public String toolName() {
		return "generateJava";
	}

	@Override
	public String description() {
		return "Generate members in a Java class, like the Source menu. action='gettersSetters' adds "
				+ "a getter and a setter for each of 'fields' (every instance field when left out); "
				+ "'kind' narrows it to getters or setters, and a final field gets no setter. "
				+ "action='constructor' adds a public constructor that takes 'fields' and assigns "
				+ "them (every instance field without an initial value when left out). "
				+ "action='overrideMethods' adds an @Override stub for each of 'methods', named as "
				+ "'toString' or 'compare(Object,Object)'; left out, for every inherited abstract "
				+ "method the class does not implement yet. A stub of an abstract method returns a "
				+ "default value, one of an implemented method calls super. Imports are added as "
				+ "needed, members that already exist are skipped and reported, and the file is "
				+ "written. 'type' is the class's fully qualified name. Refused when the file has "
				+ "unsaved changes in an open editor.";
	}

	@Override
	public JsonObject inputSchema() {
		JsonObject props = new JsonObject();
		props.add("action", ToolSupport.choice("What to generate.", ACTIONS));
		props.add("type", ToolSupport.prop("string", "Fully qualified name of the class, e.g. 'com.example.Foo'."));
		props.add("fields", ToolSupport.strings("For 'gettersSetters' / 'constructor': the field names."));
		props.add("kind", ToolSupport.choice("For 'gettersSetters': both (default), getters or setters.", KINDS));
		props.add("methods", ToolSupport.strings("For 'overrideMethods': the methods, by name or as "
				+ "'name(ParamType,…)' to pick one of several with that name."));
		return ToolSupport.schema(props, "action", "type");
	}

	/** A member to add: what it is called in the result, and its source. */
	private record Member(String label, String source) {}

	/** A field as the class declares it. */
	private record Field(String name, String type, int modifiers, boolean isBoolean, boolean hasInitialValue) {
		boolean isStatic() {
			return Modifier.isStatic(modifiers);
		}

		boolean isFinal() {
			return Modifier.isFinal(modifiers);
		}
	}

	@Override
	public McpToolResult execute(JsonObject params) {
		ClaudeCodeView.debug("[generateJava] " + params);
		try {
			String action = ToolSupport.str(params, "action");
			if (action == null || !ACTIONS.contains(action)) {
				return McpToolResult.error("'action' has to be one of: " + String.join(", ", ACTIONS) + ".");
			}
			String fqn = ToolSupport.str(params, "type");
			if (fqn == null) return McpToolResult.error("Missing required parameter: type");
			IType type = JdtUtils.findType(fqn);
			if (type == null) return McpToolResult.error("Could not resolve type: " + fqn);
			ICompilationUnit cu = type.getCompilationUnit();
			if (cu == null || !(cu.getResource() instanceof IFile file)) {
				return McpToolResult.error(fqn + " is in a library, not in a source file of the workspace.");
			}
			file.refreshLocal(IResource.DEPTH_ZERO, new NullProgressMonitor());
			if (ToolSupport.hasUnsavedChanges(file)) {
				return McpToolResult.error("The file has unsaved changes in its editor. Save or revert them first.");
			}

			ASTParser parser = ASTParser.newParser(AST.getJLSLatest());
			parser.setSource(cu);
			parser.setResolveBindings(true);
			CompilationUnit ast = (CompilationUnit) parser.createAST(new NullProgressMonitor());
			ISourceRange name = type.getNameRange();
			ASTNode named = NodeFinder.perform(ast, name.getOffset(), name.getLength());
			if (named == null || !(named.getParent() instanceof AbstractTypeDeclaration declaration)) {
				return McpToolResult.error("Could not find the declaration of " + fqn + " in its file.");
			}

			String source = cu.getSource();
			String nl = cu.findRecommendedLineSeparator();
			ImportRewrite imports = ImportRewrite.create(ast, true);
			List<Member> members = new ArrayList<>();
			JsonArray skipped = new JsonArray();
			boolean beforeMethods = false;

			if (action.equals("overrideMethods")) {
				overrides(declaration, ToolSupport.list(params, "methods"), imports, nl, members, skipped);
			} else {
				if (!(declaration instanceof TypeDeclaration cls) || cls.isInterface()) {
					return McpToolResult.error("'" + action + "' is for a class; " + fqn + " is not one.");
				}
				List<Field> fields = fields(cls, source);
				if (action.equals("constructor")) {
					members.add(constructor(cls, chosen(fields, params, false), nl));
					beforeMethods = true;
				} else {
					String kind = ToolSupport.str(params, "kind");
					if (kind == null) kind = "both";
					if (!KINDS.contains(kind)) return McpToolResult.error("'kind' has to be both, getters or setters.");
					accessors(cls, chosen(fields, params, true), kind, cu.getJavaProject(), nl, members, skipped);
				}
			}

			JsonObject result = new JsonObject();
			result.addProperty("type", fqn);
			result.addProperty("file", file.getLocation().toOSString());
			JsonArray added = new JsonArray();
			for (Member m : members) added.add(m.label());
			result.add("added", added);
			if (!skipped.isEmpty()) result.add("skipped", skipped);
			if (members.isEmpty()) return McpToolResult.success(result);

			ASTRewrite rewrite = ASTRewrite.create(ast.getAST());
			ListRewrite body = rewrite.getListRewrite(declaration, declaration.getBodyDeclarationsProperty());
			ASTNode firstMethod = null;
			if (beforeMethods) {
				for (Object o : declaration.bodyDeclarations()) {
					if (o instanceof MethodDeclaration) {
						firstMethod = (ASTNode) o;
						break;
					}
				}
			}
			for (Member m : members) {
				ASTNode placeholder = rewrite.createStringPlaceholder(m.source(), ASTNode.METHOD_DECLARATION);
				if (firstMethod != null) body.insertBefore(placeholder, firstMethod, null);
				else body.insertLast(placeholder, null);
			}

			// The members first and the imports second: the imports are above the type, so
			// their positions, worked out on the file as it was, still hold after it.
			Document document = new Document(source);
			rewrite.rewriteAST(document, cu.getJavaProject().getOptions(true)).apply(document);
			TextEdit importEdit = imports.rewriteImports(new NullProgressMonitor());
			importEdit.apply(document);

			file.setContents(new ByteArrayInputStream(document.get().getBytes(file.getCharset())),
					IResource.FORCE | IResource.KEEP_HISTORY, new NullProgressMonitor());
			return McpToolResult.success(result);
		} catch (ToolSupport.Refusal e) {
			return McpToolResult.error(e.getMessage());
		} catch (Exception | LinkageError e) {
			ClaudeCodeView.debug("[generateJava] failed: " + e);
			return McpToolResult.error("generateJava failed: " + e.getClass().getSimpleName() + ": " + e.getMessage());
		}
	}

	// ---- fields, getters and setters, constructor -------------------------------------------

	private static List<Field> fields(TypeDeclaration cls, String source) {
		List<Field> out = new ArrayList<>();
		for (FieldDeclaration declared : cls.getFields()) {
			Type type = declared.getType();
			String text = source.substring(type.getStartPosition(), type.getStartPosition() + type.getLength());
			for (Object o : declared.fragments()) {
				VariableDeclarationFragment fragment = (VariableDeclarationFragment) o;
				int dimensions = fragment.getExtraDimensions();
				boolean isBoolean = dimensions == 0 && type instanceof PrimitiveType primitive
						&& primitive.getPrimitiveTypeCode() == PrimitiveType.BOOLEAN;
				out.add(new Field(fragment.getName().getIdentifier(), text + "[]".repeat(dimensions),
						declared.getModifiers(), isBoolean, fragment.getInitializer() != null));
			}
		}
		return out;
	}

	/** The fields a call names, or the ones the action takes when it names none. */
	private static List<Field> chosen(List<Field> fields, JsonObject params, boolean forAccessors)
			throws ToolSupport.Refusal {
		List<String> names = ToolSupport.list(params, "fields");
		List<Field> out = new ArrayList<>();
		if (names.isEmpty()) {
			for (Field f : fields) {
				if (f.isStatic()) continue;
				// A final field with a value of its own cannot be assigned by a constructor.
				if (!forAccessors && f.isFinal() && f.hasInitialValue()) continue;
				out.add(f);
			}
			return out;
		}
		for (String name : names) {
			Field found = null;
			for (Field f : fields) {
				if (f.name().equals(name)) found = f;
			}
			if (found == null) {
				List<String> have = new ArrayList<>();
				for (Field f : fields) have.add(f.name());
				throw new ToolSupport.Refusal("The class has no field '" + name + "'. Its fields: "
						+ (have.isEmpty() ? "none" : String.join(", ", have)) + ".");
			}
			out.add(found);
		}
		return out;
	}

	private static void accessors(TypeDeclaration cls, List<Field> fields, String kind, IJavaProject project,
			String nl, List<Member> members, JsonArray skipped) {
		String owner = cls.getName().getIdentifier();
		for (Field f : fields) {
			String mods = f.isStatic() ? "public static " : "public ";
			if (!kind.equals("setters")) {
				String getter = NamingConventions.suggestGetterName(project, f.name(), f.modifiers(), f.isBoolean(), null);
				if (declares(cls, getter, 0)) {
					skip(skipped, getter + "()", "already there");
				} else {
					members.add(new Member(getter + "()", mods + f.type() + " " + getter + "() {" + nl
							+ "\treturn " + f.name() + ";" + nl + "}"));
				}
			}
			if (!kind.equals("getters")) {
				String setter = NamingConventions.suggestSetterName(project, f.name(), f.modifiers(), f.isBoolean(), null);
				String label = setter + "(" + f.type() + ")";
				if (f.isFinal()) {
					skip(skipped, label, "the field is final");
				} else if (declares(cls, setter, 1)) {
					skip(skipped, label, "already there");
				} else {
					members.add(new Member(label, mods + "void " + setter + "(" + f.type() + " " + f.name() + ") {" + nl
							+ "\t" + (f.isStatic() ? owner : "this") + "." + f.name() + " = " + f.name() + ";" + nl + "}"));
				}
			}
		}
	}

	private static Member constructor(TypeDeclaration cls, List<Field> fields, String nl) throws ToolSupport.Refusal {
		String owner = cls.getName().getIdentifier();
		List<String> types = new ArrayList<>();
		List<String> parameters = new ArrayList<>();
		StringBuilder assignments = new StringBuilder();
		for (Field f : fields) {
			if (f.isStatic()) throw new ToolSupport.Refusal("'" + f.name() + "' is static; a constructor does not set it.");
			if (f.isFinal() && f.hasInitialValue()) {
				throw new ToolSupport.Refusal("'" + f.name() + "' is final and already has a value.");
			}
			types.add(f.type());
			parameters.add(f.type() + " " + f.name());
			assignments.append("\tthis.").append(f.name()).append(" = ").append(f.name()).append(";").append(nl);
		}
		String label = owner + "(" + String.join(", ", types) + ")";
		for (MethodDeclaration m : cls.getMethods()) {
			if (!m.isConstructor() || m.parameters().size() != types.size()) continue;
			List<String> theirs = new ArrayList<>();
			for (Object p : m.parameters()) theirs.add(((SingleVariableDeclaration) p).getType().toString());
			if (theirs.equals(types)) throw new ToolSupport.Refusal("The class already has the constructor " + label + ".");
		}
		return new Member(label, "public " + owner + "(" + String.join(", ", parameters) + ") {" + nl + assignments + "}");
	}

	private static boolean declares(TypeDeclaration cls, String method, int parameterCount) {
		for (MethodDeclaration m : cls.getMethods()) {
			if (m.getName().getIdentifier().equals(method) && m.parameters().size() == parameterCount) return true;
		}
		return false;
	}

	private static void skip(JsonArray skipped, String member, String reason) {
		JsonObject s = new JsonObject();
		s.addProperty("member", member);
		s.addProperty("reason", reason);
		skipped.add(s);
	}

	// ---- overriding stubs -------------------------------------------------------------------

	private static void overrides(AbstractTypeDeclaration declaration, List<String> wanted, ImportRewrite imports,
			String nl, List<Member> members, JsonArray skipped) throws ToolSupport.Refusal {
		ITypeBinding type = declaration.resolveBinding();
		if (type == null) throw new ToolSupport.Refusal("The type could not be resolved; fix its errors first.");
		List<IMethodBinding> inherited = inherited(type);

		List<IMethodBinding> picked = new ArrayList<>();
		if (wanted.isEmpty()) {
			for (IMethodBinding m : inherited) {
				if (Modifier.isAbstract(m.getModifiers())) picked.add(m);
			}
		} else {
			for (String name : wanted) {
				int before = picked.size();
				for (IMethodBinding m : inherited) {
					if (named(m, name) && !picked.contains(m)) picked.add(m);
				}
				if (picked.size() == before) {
					List<String> have = new ArrayList<>();
					for (IMethodBinding m : inherited) {
						if (have.size() < 40 && !have.contains(m.getName())) have.add(m.getName());
					}
					throw new ToolSupport.Refusal("Nothing named '" + name + "' is there to override, or the "
							+ "class already overrides it. It can override: " + String.join(", ", have) + ".");
				}
			}
		}

		List<ITypeBinding> direct = new ArrayList<>();
		for (ITypeBinding i : type.getInterfaces()) direct.add(i.getErasure());
		for (IMethodBinding m : picked) {
			boolean isAbstract = Modifier.isAbstract(m.getModifiers());
			ITypeBinding owner = m.getDeclaringClass();
			if (!isAbstract && owner.isInterface() && !direct.contains(owner.getErasure())) {
				skip(skipped, signature(m), "a default method of " + owner.getName()
						+ ", which the class does not implement directly, so a stub could not call it");
				continue;
			}
			members.add(new Member(signature(m), stub(m, isAbstract, imports, nl)));
		}
	}

	/**
	 * What the type could override: the visible, non-final instance methods of its
	 * superclasses and interfaces that it does not override already. Where several of them
	 * are the same method, the one found first stands for it — a superclass's before an
	 * interface's — so an interface method a superclass implements is not listed as abstract.
	 */
	private static List<IMethodBinding> inherited(ITypeBinding type) {
		List<ITypeBinding> supertypes = new ArrayList<>();
		supertypes(type, supertypes);
		List<IMethodBinding> out = new ArrayList<>();
		IMethodBinding[] own = type.getDeclaredMethods();
		for (ITypeBinding supertype : supertypes) {
			for (IMethodBinding m : supertype.getDeclaredMethods()) {
				int mods = m.getModifiers();
				if (m.isConstructor() || m.isSynthetic() || Modifier.isStatic(mods) || Modifier.isPrivate(mods)
						|| Modifier.isFinal(mods)) {
					continue;
				}
				boolean packagePrivate = !Modifier.isPublic(mods) && !Modifier.isProtected(mods) && !supertype.isInterface();
				if (packagePrivate && !samePackage(supertype, type)) continue;
				if (same(m, Arrays.asList(own)) || same(m, out)) continue;
				out.add(m);
			}
		}
		return out;
	}

	private static void supertypes(ITypeBinding type, List<ITypeBinding> out) {
		ITypeBinding superclass = type.getSuperclass();
		if (superclass != null && !out.contains(superclass)) {
			out.add(superclass);
			supertypes(superclass, out);
		}
		for (ITypeBinding i : type.getInterfaces()) {
			if (out.contains(i)) continue;
			out.add(i);
			supertypes(i, out);
		}
	}

	private static boolean same(IMethodBinding method, List<IMethodBinding> among) {
		for (IMethodBinding other : among) {
			if (other.overrides(method) || (other.getName().equals(method.getName()) && other.isSubsignature(method))) {
				return true;
			}
		}
		return false;
	}

	private static boolean samePackage(ITypeBinding a, ITypeBinding b) {
		return a.getPackage() != null && b.getPackage() != null
				&& a.getPackage().getName().equals(b.getPackage().getName());
	}

	/** Whether a method is the one a call names: 'run', or 'compare(Object,Object)'. */
	private static boolean named(IMethodBinding method, String wanted) {
		int paren = wanted.indexOf('(');
		if (paren < 0) return method.getName().equals(wanted);
		if (!method.getName().equals(wanted.substring(0, paren).trim()) || !wanted.endsWith(")")) return false;
		String[] types = JdtUtils.parseParamTypes(wanted.substring(paren + 1, wanted.length() - 1));
		ITypeBinding[] theirs = method.getParameterTypes();
		if (types.length != theirs.length) return false;
		for (int i = 0; i < types.length; i++) {
			String simple = JdtUtils.simpleName(JdtUtils.stripGenerics(types[i]));
			if (!simple.equals(theirs[i].getErasure().getName())) return false;
		}
		return true;
	}

	private static String signature(IMethodBinding method) {
		List<String> types = new ArrayList<>();
		for (ITypeBinding p : method.getParameterTypes()) types.add(p.getErasure().getName());
		return method.getName() + "(" + String.join(",", types) + ")";
	}

	private static String stub(IMethodBinding method, boolean isAbstract, ImportRewrite imports, String nl) {
		StringBuilder b = new StringBuilder("@Override").append(nl);
		int mods = method.getModifiers();
		// An interface's methods are public whether or not they say so.
		if (Modifier.isPublic(mods) || method.getDeclaringClass().isInterface()) b.append("public ");
		else if (Modifier.isProtected(mods)) b.append("protected ");

		ITypeBinding[] typeParameters = method.getTypeParameters();
		if (typeParameters.length > 0) {
			List<String> declared = new ArrayList<>();
			for (ITypeBinding parameter : typeParameters) {
				List<String> bounds = new ArrayList<>();
				for (ITypeBinding bound : parameter.getTypeBounds()) bounds.add(imports.addImport(bound));
				declared.add(parameter.getName() + (bounds.isEmpty() ? "" : " extends " + String.join(" & ", bounds)));
			}
			b.append("<").append(String.join(", ", declared)).append("> ");
		}

		ITypeBinding returns = method.getReturnType();
		b.append(imports.addImport(returns)).append(" ").append(method.getName()).append("(");

		ITypeBinding[] types = method.getParameterTypes();
		String[] names = parameterNames(method, types.length);
		for (int i = 0; i < types.length; i++) {
			if (i > 0) b.append(", ");
			boolean varargs = method.isVarargs() && i == types.length - 1 && types[i].isArray();
			b.append(varargs ? imports.addImport(types[i].getComponentType()) + "..." : imports.addImport(types[i]));
			b.append(" ").append(names[i]);
		}
		b.append(")");

		ITypeBinding[] thrown = method.getExceptionTypes();
		if (thrown.length > 0) {
			List<String> exceptions = new ArrayList<>();
			for (ITypeBinding e : thrown) exceptions.add(imports.addImport(e));
			b.append(" throws ").append(String.join(", ", exceptions));
		}
		b.append(" {").append(nl);

		boolean isVoid = "void".equals(returns.getName());
		if (isAbstract) {
			if (!isVoid) {
				String value = !returns.isPrimitive() ? "null" : "boolean".equals(returns.getName()) ? "false" : "0";
				b.append("\treturn ").append(value).append(";").append(nl);
			}
		} else {
			ITypeBinding owner = method.getDeclaringClass();
			String target = owner.isInterface() ? imports.addImport(owner.getErasure()) + ".super" : "super";
			b.append("\t").append(isVoid ? "" : "return ").append(target).append(".").append(method.getName())
					.append("(").append(String.join(", ", names)).append(");").append(nl);
		}
		return b.append("}").toString();
	}

	/** The names the overridden method gives its parameters, where they are known. */
	private static String[] parameterNames(IMethodBinding method, int count) {
		String[] names = new String[count];
		for (int i = 0; i < count; i++) names[i] = "arg" + i;
		try {
			if (method.getMethodDeclaration().getJavaElement() instanceof IMethod element) {
				String[] theirs = element.getParameterNames();
				if (theirs.length == count) return theirs;
			}
		} catch (Exception e) {
			// no source and no debug information: arg0, arg1, …
		}
		return names;
	}
}
