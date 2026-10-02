"""Map AST reads to compiler scopes without executing checked source."""

import ast
import symtable


class ScopeCollector(ast.NodeVisitor):
    """Collect lexical reads and imports using the compiler symbol tables.

    Args:
        text_str: Compilable Python source.
        path_str: Diagnostic source label.
    Returns:
        ScopeCollector: Stateful visitor for a single source snapshot.
    Warnings:
        This records binding existence, not initialization or execution flow.
    """

    def __init__(self, text_str: str, path_str: str) -> None:
        """Initialize compiler tables and collected source evidence.

        Args:
            text_str (str): Compilable source.
            path_str (str): Source label.
        Returns:
            None: Initializes visitor state.
        Warnings:
            Compilation failures propagate without executing source.
        """
        self.root_info = symtable.symtable(text_str, path_str, "exec")
        self.stack_list = [self.root_info]
        self.parents_dict = {}
        self.tables_dict = {self.root_info.get_id(): self.root_info}
        self.reads_list = []
        self.imports_list = []
        self.used_tables_set = set()
        self.annotation_bool = False
        self.class_str = ""
        self.comprehension_names_list = []
        self.walrus_globals_set = set()

    def visit_Name(self, node: ast.Name) -> None:
        """Record the compiler scope of a loaded identifier.

        Args:
            node (ast.Name): Name expression.
        Returns:
            None: Adds source evidence for loads.
        Warnings:
            Annotation reads have separate diagnostic policy.
        """
        if isinstance(node.ctx, ast.Load):
            name_str = node.id
            if name_str.startswith("__") and not name_str.endswith("__"):
                prefix_str = self.class_str.lstrip("_")
                if prefix_str:
                    name_str = "_" + prefix_str + name_str
            self.reads_list.append((node, self.stack_list[-1], name_str,
                                    self.annotation_bool, any(
                                        node.id in names_set for names_set in
                                        self.comprehension_names_list)))

    def collect_scope_none(self, node: ast.AST, name_str: str,
                           body_list: list[ast.AST]) -> None:
        """Visit a body in its matching compiler child scope.

        Args:
            node (ast.AST): Scope-creating expression or statement.
            name_str (str): Compiler scope name.
            body_list (list[ast.AST]): Nodes evaluated inside that scope.
        Returns:
            None: Appends reads and imports in lexical context.
        Warnings:
            Unsupported compiler scope mapping raises a refusal.
        """
        parent_info = self.stack_list[-1]
        matches_list = [child_info for child_info in parent_info.get_children()
                        if child_info.get_name() == name_str
                        and child_info.get_lineno() == node.lineno
                        and child_info.get_id() not in self.used_tables_set]
        if not matches_list:
            raise ValueError(
                f"Unresolved compiler scope at line {node.lineno}")
        child_info = matches_list[0]
        self.used_tables_set.add(child_info.get_id())
        self.parents_dict[child_info.get_id()] = parent_info
        self.tables_dict[child_info.get_id()] = child_info
        self.stack_list.append(child_info)
        for child_node in body_list:
            self.visit(child_node)
        self.stack_list.pop()

    def collect_annotation_none(self, node: ast.AST | None) -> None:
        """Record annotation uses separately from runtime value loads.

        Args:
            node (ast.AST | None): Optional annotation expression.
        Returns:
            None: Collects annotation references for import usage.
        Warnings:
            Forward annotation names are not undefined-name diagnostics.
        """
        if node is not None:
            previous_bool = self.annotation_bool
            self.annotation_bool = True
            self.visit(node)
            self.annotation_bool = previous_bool

    def collect_function_none(self, node: ast.AST) -> None:
        """Visit defaults in the parent and function bodies in child scopes.

        Args:
            node (ast.AST): Function, async function or lambda.
        Returns:
            None: Collects lexical reads in evaluation scopes.
        Warnings:
            Decorators and defaults execute outside the body scope.
        """
        for expression_node in getattr(node, "decorator_list", []):
            self.visit(expression_node)
        for expression_node in [*node.args.defaults, *node.args.kw_defaults]:
            if expression_node is not None:
                self.visit(expression_node)
        for argument_node in [*node.args.posonlyargs, *node.args.args,
                              *node.args.kwonlyargs, node.args.vararg,
                              node.args.kwarg]:
            if argument_node is not None:
                self.collect_annotation_none(argument_node.annotation)
        self.collect_annotation_none(getattr(node, "returns", None))
        body_list = [node.body] if isinstance(node, ast.Lambda) else node.body
        self.collect_scope_none(node, getattr(node, "name", "lambda"),
                                 body_list)

    visit_FunctionDef = collect_function_none
    visit_AsyncFunctionDef = collect_function_none
    visit_Lambda = collect_function_none

    def visit_ClassDef(self, node: ast.ClassDef) -> None:
        """Visit class setup outside the class namespace and its body inside.

        Args:
            node (ast.ClassDef): Class definition.
        Returns:
            None: Records lexical scope evidence.
        Warnings:
            Methods do not close over ordinary class-local bindings.
        """
        for expression_node in [*node.decorator_list, *node.bases,
                                *(entry.value for entry in node.keywords)]:
            self.visit(expression_node)
        previous_str = self.class_str
        self.class_str = node.name
        self.collect_scope_none(node, node.name, node.body)
        self.class_str = previous_str

    def collect_comprehension_none(self, node: ast.AST) -> None:
        """Visit the first iterable outside the comprehension scope.

        Args:
            node (ast.AST): List, set, dictionary or generator expression.
        Returns:
            None: Records inner and outer lexical loads.
        Warnings:
            Walrus bindings use the compiler's free/global classification.
        """
        self.visit(node.generators[0].iter)
        body_list = []
        for index_int, generator_node in enumerate(node.generators):
            if index_int:
                body_list.append(generator_node.iter)
            body_list.extend(generator_node.ifs)
        body_list.extend(
            [node.key, node.value] if isinstance(node, ast.DictComp)
            else [node.elt])
        if isinstance(node, ast.GeneratorExp):
            self.collect_scope_none(node, "genexpr", body_list)
        else:
            self.collect_inline_none(node, body_list)

    def collect_inline_none(self, node: ast.AST,
                            body_list: list[ast.AST]) -> None:
        """Track isolated names in Python's inlined comprehensions.

        Args:
            node (ast.AST): List, set or dictionary comprehension.
            body_list (list[ast.AST]): Expressions inside the comprehension.
        Returns:
            None: Collects reads with isolated target-name evidence.
        Warnings:
            Conditional initialization remains outside this analysis.
        """
        names_set = {target_node.id for generator_node in node.generators
                     for target_node in ast.walk(generator_node.target)
                     if isinstance(target_node, ast.Name)}
        self.comprehension_names_list.append(names_set)
        original_list = self.stack_list
        if self.stack_list[-1].get_type() == "class":
            self.stack_list = [table_info for table_info in self.stack_list
                               if table_info.get_type() != "class"]
        for expression_node in body_list:
            self.visit(expression_node)
        self.stack_list = original_list
        self.comprehension_names_list.pop()

    visit_ListComp = collect_comprehension_none
    visit_SetComp = collect_comprehension_none
    visit_DictComp = collect_comprehension_none
    visit_GeneratorExp = collect_comprehension_none

    def visit_AnnAssign(self, node: ast.AnnAssign) -> None:
        """Keep annotated assignment value and annotation reads distinct.

        Args:
            node (ast.AnnAssign): Annotated assignment.
        Returns:
            None: Adds available reads.
        Warnings:
            Annotation-only targets do not establish initialized values.
        """
        self.collect_annotation_none(node.annotation)
        self.visit(node.target)
        if node.value is not None:
            self.visit(node.value)

    def collect_import_none(self, node: ast.Import | ast.ImportFrom) -> None:
        """Record imported binding names and explicit re-export evidence.

        Args:
            node (ast.Import | ast.ImportFrom): Import statement.
        Returns:
            None: Adds aliases with compiler scope and source location.
        Warnings:
            Import side effects and external exports remain unresolved.
        """
        if isinstance(node, ast.ImportFrom) and node.module == "__future__":
            return
        for alias_node in node.names:
            name_str = alias_node.asname or alias_node.name.split(".")[0]
            self.imports_list.append((
                alias_node, self.stack_list[-1], name_str,
                alias_node.asname == alias_node.name))

    visit_Import = collect_import_none
    visit_ImportFrom = collect_import_none


    def visit_NamedExpr(self, node: ast.NamedExpr) -> None:
        """Record module bindings created by comprehension assignment.

        Args:
            node (ast.NamedExpr): Assignment expression.
        Returns:
            None: Records binding existence and visits the value.
        Warnings:
            The comprehension may execute zero times.
        """
        table_info = self.stack_list[-1]
        if table_info is self.root_info or (
            node.target.id in table_info.get_identifiers()
            and table_info.lookup(node.target.id).is_global()
        ):
            self.walrus_globals_set.add(node.target.id)
        self.visit(node.value)
