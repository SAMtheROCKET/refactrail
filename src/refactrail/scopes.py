"""Collect the names each scope binds, as the naming rules need them."""

from dataclasses import dataclass, field
import ast

from refactrail.source import (
    SourceFile, locate_definition_name_tuple, locate_node_tuple,
)

FUNCTION_NODES_TUPLE = (ast.FunctionDef, ast.AsyncFunctionDef)
COMPREHENSION_NODES_TUPLE = (ast.ListComp, ast.SetComp, ast.DictComp,
                             ast.GeneratorExp)


@dataclass
class Binding:
    """Describe the earliest binding of a name in one scope.

    Args:
        name: Bound name.
        kind: "function", "class", "parameter", "variable", "import" or
            "except".
        scope_kind: "module", "class", "function", "lambda" or
            "comprehension".
        line: 1-based line.
        column: 1-based character column.
        value_node: Assigned value_node for a plain single-name assignment.
        implicit: True for the self/cls parameter of a method.
        imported_name: Last part of the imported name, for imports.
    Returns:
        Binding: One record per name and scope.
    Warnings:
        Names declared global or nonlocal are not bindings of the scope.
    """

    name: str
    kind: str
    scope_kind: str
    line: int
    column: int
    value_node: ast.expr | None = None
    implicit: bool = False
    imported_name: str = ""


@dataclass
class Scope:
    """Hold the bindings of one scope while the tree is walked.

    Args:
        kind: Scope kind, as in Binding.scope_kind.
        excluded: Names declared global or nonlocal here.
        parent: Enclosing scope; None for the module.
        bindings: Name to its earliest binding.
    Returns:
        Scope: Mutable collection state.
    Warnings:
        Only the earliest position of each name is kept.
    """

    kind: str
    excluded: set[str] = field(default_factory=set)
    parent: "Scope | None" = None
    bindings: dict[str, Binding] = field(default_factory=dict)


def collect_declared_set(statements_list: list[ast.stmt]) -> set[str]:
    """List names declared global or nonlocal in a function body.

    Args:
        statements_list (list[ast.stmt]): Function body.
    Returns:
        set[str]: Declared names.
    Warnings:
        Nested functions and classes are not entered.
    """
    names_set: set[str] = set()
    pending_list: list[ast.AST] = list(statements_list)
    while pending_list:
        node = pending_list.pop()
        if isinstance(node, (ast.Global, ast.Nonlocal)):
            names_set.update(node.names)
        elif not isinstance(node, (*FUNCTION_NODES_TUPLE, ast.ClassDef,
                                   ast.Lambda)):
            pending_list.extend(ast.iter_child_nodes(node))
    return names_set


class BindingCollector:
    """Walk a module and record the earliest binding of each name per scope.

    Args:
        source_info (SourceFile): File being analysed.
    Returns:
        BindingCollector: Call collect_bindings_list() for the result.
    Warnings:
        match-statement captures are not collected (see RULES.md).
    """

    def __init__(self, source_info: SourceFile) -> None:
        """Prepare an empty collector.

        Args:
            source_info (SourceFile): File being analysed.
        Returns:
            None: Stores the file and an empty scope list.
        Warnings:
            None.
        """
        self.source_info = source_info
        self.scopes_list: list[Scope] = []

    def collect_bindings_list(self, tree: ast.Module) -> list[Binding]:
        """Return every scope's bindings in position order.

        Args:
            tree (ast.Module): Parsed module.
        Returns:
            list[Binding]: Earliest binding per name and scope.
        Warnings:
            Each call walks the whole tree again.
        """
        self.scopes_list = []
        module_scope = self.open_scope(("module", set()), None)
        self.walk_statements_none(tree.body, module_scope)
        return sorted((binding for scope in self.scopes_list
                       for binding in scope.bindings.values()),
                      key=lambda binding: (binding.line, binding.column,
                                           binding.name))

    def open_scope(
        self, kind_tuple: tuple[str, set[str]], parent: Scope | None,
    ) -> Scope:
        """Create and remember a new scope.

        Args:
            kind_tuple (tuple[str, set[str]]): Kind and excluded names.
            parent (Scope | None): Enclosing scope.
        Returns:
            Scope: The new scope.
        Warnings:
            None.
        """
        scope = Scope(kind_tuple[0], kind_tuple[1], parent)
        self.scopes_list.append(scope)
        return scope

    def record_none(self, scope: Scope, binding: Binding) -> None:
        """Keep a binding if it is the earliest one of its name.

        Args:
            scope (Scope): Scope that binds the name.
            binding (Binding): Candidate binding.
        Returns:
            None: Updates the scope in place.
        Warnings:
            Declared global/nonlocal names are skipped.
        """
        if binding.name in scope.excluded:
            return
        current = scope.bindings.get(binding.name)
        if current is None or (binding.line, binding.column) < (
            current.line, current.column,
        ):
            scope.bindings[binding.name] = binding

    def walk_statements_none(
        self, statements_list: list[ast.stmt], scope: Scope,
    ) -> None:
        """Walk statements in one scope.

        Args:
            statements_list (list[ast.stmt]): Statements to walk.
            scope (Scope): Scope they belong to.
        Returns:
            None: Records bindings.
        Warnings:
            None.
        """
        for statement_node in statements_list:
            self.walk_node_none(statement_node, scope)

    def walk_node_none(self, node: ast.AST, scope: Scope) -> None:
        """Dispatch one node to the handler for its kind.

        Args:
            node (ast.AST): Node to walk.
            scope (Scope): Current scope.
        Returns:
            None: Records bindings.
        Warnings:
            Unhandled node kinds are walked child by child.
        """
        if isinstance(node, FUNCTION_NODES_TUPLE):
            self.walk_function_none(node, scope)
        elif isinstance(node, ast.ClassDef):
            self.walk_class_none(node, scope)
        elif isinstance(node, ast.Lambda):
            self.walk_lambda_none(node, scope)
        elif isinstance(node, COMPREHENSION_NODES_TUPLE):
            self.walk_comprehension_none(node, scope)
        elif isinstance(node, (ast.Assign, ast.AnnAssign, ast.NamedExpr)):
            self.walk_assignment_none(node, scope)
        elif isinstance(node, (ast.Import, ast.ImportFrom)):
            self.record_imports_none(node, scope)
        elif isinstance(node, ast.ExceptHandler):
            self.walk_handler_none(node, scope)
        elif isinstance(node, ast.Name) and isinstance(node.ctx, ast.Store):
            self.record_name_none(node, scope, None)
        else:
            for child_node in ast.iter_child_nodes(node):
                self.walk_node_none(child_node, scope)

    def record_name_none(
        self, node: ast.Name, scope: Scope, value_node: ast.expr | None,
    ) -> None:
        """Record a stored name, with its value when it is the only target.

        Args:
            node (ast.Name): Stored name.
            scope (Scope): Scope that binds it.
            value_node (ast.expr | None): Assigned value_node, if any.
        Returns:
            None: Records the binding.
        Warnings:
            None.
        """
        line_int, column_int = locate_node_tuple(self.source_info, node)
        self.record_none(scope, Binding(node.id, "variable", scope.kind,
                                        line_int, column_int, value_node))

    def walk_assignment_none(self, node: ast.AST, scope: Scope) -> None:
        """Walk =, annotated assignment or :=, keeping single-name values.

        Args:
            node (ast.AST): Assign, AnnAssign or NamedExpr.
            scope (Scope): Current scope.
        Returns:
            None: Records bindings.
        Warnings:
            := binds in the nearest scope that is not a comprehension.
        """
        value_node = getattr(node, "value", None)
        if value_node is not None:
            self.walk_node_none(value_node, scope)
        if isinstance(node, ast.AnnAssign):
            self.walk_node_none(node.annotation, scope)
        targets_list = (node.targets if isinstance(node, ast.Assign)
                        else [node.target])
        target_scope = (self.find_walrus_scope(scope)
                        if isinstance(node, ast.NamedExpr) else scope)
        if len(targets_list) == 1 and isinstance(targets_list[0], ast.Name):
            self.record_name_none(targets_list[0], target_scope, value_node)
            return
        for target_node in targets_list:
            self.walk_node_none(target_node, target_scope)

    def find_walrus_scope(self, scope: Scope) -> Scope:
        """Find where a := target is bound.

        Args:
            scope (Scope): Scope holding the := expression.
        Returns:
            Scope: The nearest enclosing scope that is not a
                comprehension.
        Warnings:
            None.
        """
        while scope.kind == "comprehension" and scope.parent is not None:
            scope = scope.parent
        return scope

    def record_imports_none(self, node: ast.AST, scope: Scope) -> None:
        """Record the names an import statement binds.

        Args:
            node (ast.AST): Import or ImportFrom.
            scope (Scope): Current scope.
        Returns:
            None: Records bindings at each alias.
        Warnings:
            Star imports bind nothing that can be named here.
        """
        for alias_node in node.names:
            if alias_node.name == "*":
                continue
            bound_str = alias_node.asname or alias_node.name.split(".")[0]
            line_int, column_int = locate_node_tuple(self.source_info,
                                                     alias_node)
            self.record_none(scope, Binding(
                bound_str, "import", scope.kind, line_int, column_int,
                imported_name=alias_node.name.split(".")[-1]))

    def walk_handler_none(self, node: ast.ExceptHandler, scope: Scope) -> None:
        """Walk an except clause and record its 'as' name.

        Args:
            node (ast.ExceptHandler): Handler.
            scope (Scope): Current scope.
        Returns:
            None: Records bindings.
        Warnings:
            The name is reported at the 'except' keyword.
        """
        if node.name:
            line_int, column_int = locate_node_tuple(self.source_info, node)
            self.record_none(scope, Binding(node.name, "except", scope.kind,
                                            line_int, column_int))
        for child_node in ast.iter_child_nodes(node):
            self.walk_node_none(child_node, scope)

    def walk_function_none(self, node: ast.AST, scope: Scope) -> None:
        """Record a function, then walk its parameters and body.

        Args:
            node (ast.AST): FunctionDef or AsyncFunctionDef.
            scope (Scope): Enclosing scope.
        Returns:
            None: Records bindings in both scopes.
        Warnings:
            Decorators, defaults and annotations belong to the enclosing
            scope.
        """
        arguments_node = node.args
        for child_node in [*node.decorator_list, *arguments_node.defaults,
                           *filter(None, arguments_node.kw_defaults)]:
            self.walk_node_none(child_node, scope)
        line_int, column_int = locate_definition_name_tuple(
            self.source_info, node)
        self.record_none(scope, Binding(node.name, "function", scope.kind,
                                        line_int, column_int))
        inner_scope = self.open_scope(
            ("function", collect_declared_set(node.body)), scope)
        implicit_bool = scope.kind == "class" and not any(
            getattr(decorator_node, "id", None) == "staticmethod"
            for decorator_node in node.decorator_list)
        self.record_parameters_none(arguments_node, inner_scope,
                                    implicit_bool)
        self.walk_statements_none(node.body, inner_scope)

    def record_parameters_none(
        self, arguments_node: ast.arguments, scope: Scope,
        implicit_bool: bool,
    ) -> None:
        """Record every parameter of a function or lambda.

        Args:
            arguments_node (ast.arguments): Parameters.
            scope (Scope): The function's own scope.
            implicit_bool (bool): Whether the first one is self/cls.
        Returns:
            None: Records parameter bindings.
        Warnings:
            None.
        """
        parameters_list = [*arguments_node.posonlyargs, *arguments_node.args,
                           arguments_node.vararg, *arguments_node.kwonlyargs,
                           arguments_node.kwarg]
        positional_list = [*arguments_node.posonlyargs, *arguments_node.args]
        for parameter_node in filter(None, parameters_list):
            line_int, column_int = locate_node_tuple(self.source_info,
                                                     parameter_node)
            self.record_none(scope, Binding(
                parameter_node.arg, "parameter", scope.kind, line_int,
                column_int, implicit=implicit_bool and bool(positional_list)
                and parameter_node is positional_list[0]))

    def walk_class_none(self, node: ast.ClassDef, scope: Scope) -> None:
        """Record a class, then walk its body in a class scope.

        Args:
            node (ast.ClassDef): Class definition.
            scope (Scope): Enclosing scope.
        Returns:
            None: Records bindings.
        Warnings:
            Decorators, bases and keywords belong to the enclosing scope.
        """
        for child_node in [*node.decorator_list, *node.bases, *(
                keyword_node.value for keyword_node in node.keywords)]:
            self.walk_node_none(child_node, scope)
        line_int, column_int = locate_definition_name_tuple(
            self.source_info, node)
        self.record_none(scope, Binding(node.name, "class", scope.kind,
                                        line_int, column_int))
        self.walk_statements_none(
            node.body, self.open_scope(("class", set()), scope))

    def walk_lambda_none(self, node: ast.Lambda, scope: Scope) -> None:
        """Walk a lambda's defaults, parameters and body.

        Args:
            node (ast.Lambda): Lambda expression.
            scope (Scope): Enclosing scope.
        Returns:
            None: Records bindings.
        Warnings:
            None.
        """
        for child_node in [*node.args.defaults,
                           *filter(None, node.args.kw_defaults)]:
            self.walk_node_none(child_node, scope)
        inner_scope = self.open_scope(("lambda", set()), scope)
        self.record_parameters_none(node.args, inner_scope, False)
        self.walk_node_none(node.body, inner_scope)

    def walk_comprehension_none(self, node: ast.AST, scope: Scope) -> None:
        """Walk a comprehension in its own scope.

        Args:
            node (ast.AST): List, set, dict comprehension or generator.
            scope (Scope): Enclosing scope.
        Returns:
            None: Records bindings.
        Warnings:
            The first iterable is evaluated in the enclosing scope.
        """
        generators_list = node.generators
        self.walk_node_none(generators_list[0].iter, scope)
        inner_scope = self.open_scope(("comprehension", set()), scope)
        for index_int, generator_node in enumerate(generators_list):
            if index_int:
                self.walk_node_none(generator_node.iter, inner_scope)
            self.walk_node_none(generator_node.target, inner_scope)
            for condition_node in generator_node.ifs:
                self.walk_node_none(condition_node, inner_scope)
        for part_name_str in ("elt", "key", "value"):
            part_node = getattr(node, part_name_str, None)
            if part_node is not None:
                self.walk_node_none(part_node, inner_scope)
