"""The flow-ordered checker behind the Pyflakes-compatible scope codes.

See compat_bindings for the model. This module walks a module in
execution order and calls a report function for F401, F402, F403, F405,
F406, F811, F821, F822, F823, F841 and F842 observations.
"""

import ast
from collections import deque
from collections.abc import Callable
from pathlib import Path
import re
import sys

from refactrail.compat_pycodestyle import find_identifier_tuple
from refactrail.rules.context import RuleContext
from refactrail.source import (
    locate_definition_name_tuple, locate_node_tuple,
)
from refactrail.compat_bindings import (
    ANNOTATION_KIND_STR, ARGUMENT_KIND_STR, ASSIGNMENT_KIND_STR,
    BUILTIN_NAMES_FROZENSET, CLASS_KIND_STR, CLASS_NAMES_FROZENSET,
    CLASS_SCOPE_STR, COMPREHENSION_SCOPE_STR, DECLARATION_KIND_STR,
    DEFINITION_KIND_STR, FUNCTION_SCOPE_STR, FUTURE_KIND_STR,
    HANDLER_KIND_STR, IMPORT_KIND_STR, LOOP_KIND_STR, MODULE_SCOPE_STR,
    TYPE_SCOPE_STR, UNPACKED_KIND_STR, Binding, Scope,
)

SCOPE_CODES_TUPLE = ("F401", "F402", "F403", "F405", "F406", "F811",
                     "F821", "F822", "F823", "F841", "F842")
DUMMY_NAME_PATTERN = re.compile(r"^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$")
TRACEBACK_NAMES_FROZENSET = frozenset((
    "__tracebackhide__", "__traceback_info__", "__traceback_supplement__",
    "__debuggerskip__"))
REDEFINABLE_KINDS_FROZENSET = frozenset((
    DEFINITION_KIND_STR, CLASS_KIND_STR, IMPORT_KIND_STR))
FUNCTION_NODES_TUPLE = (ast.FunctionDef, ast.AsyncFunctionDef)
COMPREHENSION_NODES_TUPLE = (ast.ListComp, ast.SetComp, ast.DictComp,
                             ast.GeneratorExp)


class ScopeChecker:
    """Walk one module and report scope observations through a callback."""

    def __init__(self, tree: ast.Module, report: Callable[..., None],
                 is_init_bool: bool, folder: Path | None = None) -> None:
        """Prepare the walk.

        Args:
            tree (ast.Module): Parsed module.
            report: Callable(code, node, message) adding a finding.
            is_init_bool (bool): Whether the file is a package __init__.
            folder (Path | None): The file's folder, for submodules.
        Returns:
            None: Initialises the checker.
        Warnings:
            None.
        """
        self.tree = tree
        self.report = report
        self.is_init_bool = is_init_bool
        self.folder = folder
        self.module_scope = Scope(MODULE_SCOPE_STR)
        self.stack_list = [self.module_scope]
        self.branch_tuple: tuple = ()
        self.deferred_queue: deque = deque()
        self.try_ids_set: set[int] = set()
        self.finished_scopes_list: list[Scope] = []
        self.redefinitions_list: list[tuple[Binding, Binding]] = []
        self.in_type_checking_bool = False
        self.handled_names_list: list[set[str]] = []
        self.future_annotations_bool = any(
            isinstance(node, ast.ImportFrom) and node.module == "__future__"
            and any(alias.name == "annotations" for alias in node.names)
            for node in tree.body)
        self.all_names_list: list[tuple[str, ast.AST]] = []
        self.loaded_names_set = {node.id for node in ast.walk(tree)
                                 if isinstance(node, ast.Name)
                                 and isinstance(node.ctx, ast.Load)}
        self.declare_globals_none(tree)

    def declare_globals_none(self, tree: ast.Module) -> None:
        """Bind every name a `global` statement declares at module level.

        Args:
            tree (ast.Module): Parsed module.
        Returns:
            None: Adds used declaration bindings to the module scope.
        Warnings:
            Existing module bindings are kept.
        """
        for node in ast.walk(tree):
            if isinstance(node, ast.Global):
                for name_str in node.names:
                    declared = Binding(name_str, DECLARATION_KIND_STR, node)
                    declared.is_used = True
                    self.module_scope.bindings_dict.setdefault(name_str,
                                                               declared)

    # ----- running -----

    def run_none(self) -> None:
        """Walk the module, then deferred bodies, then report unused.

        Returns:
            None: Calls the report function.
        Warnings:
            None.
        """
        self.visit_statements_none(self.tree.body)
        while self.deferred_queue:
            callback, stack_list, branch_tuple = (
                self.deferred_queue.popleft())
            saved_tuple = (self.stack_list, self.branch_tuple)
            self.stack_list, self.branch_tuple = list(stack_list), (
                branch_tuple)
            callback()
            self.stack_list, self.branch_tuple = saved_tuple
        for scope in self.finished_scopes_list:
            self.check_function_scope_none(scope)
        for shadowed, binding in self.redefinitions_list:
            if not shadowed.is_used:
                self.report("F811", binding.node,
                            f"Redefinition of unused `{binding.name}` from "
                            f"line {shadowed.node.lineno}.", shadowed.node)
        self.check_exports_none()
        self.check_unused_imports_none(self.module_scope)

    def schedule_none(self, callback: Callable[[], None]) -> None:
        """Run a callback after the module, in the current context.

        Args:
            callback: Function without arguments.
        Returns:
            None: Queues it with the current scope chain.
        Warnings:
            None.
        """
        self.deferred_queue.append((callback, list(self.stack_list),
                                    self.branch_tuple))

    @property
    def scope(self) -> Scope:
        """The innermost scope.

        Returns:
            Scope: The current scope.
        Warnings:
            None.
        """
        return self.stack_list[-1]

    # ----- bindings -----

    def add_binding_none(self, binding: Binding) -> None:
        """Bind a name in the current scope (or the module for globals).

        Args:
            binding (Binding): The new binding.
        Returns:
            None: Reports F402 and F811 where they apply.
        Warnings:
            None.
        """
        scope = self.scope
        is_redirected_bool = False
        if binding.name in scope.nonlocal_dict:
            scope = scope.nonlocal_dict[binding.name]
            is_redirected_bool = binding.is_used = True
        elif binding.name in scope.globals_set:
            scope = self.module_scope
            binding.is_used = binding.name in self.loaded_names_set or (
                binding.kind == IMPORT_KIND_STR)
            is_redirected_bool = True
        binding.branch = self.branch_tuple
        binding.scope = scope
        binding.is_typing_only = self.in_type_checking_bool
        existing = scope.bindings_dict.get(binding.name)
        scope.history_list.append(binding)
        if existing is None or existing.kind == DECLARATION_KIND_STR:
            if not is_redirected_bool:
                self.check_outer_shadowing_none(binding)
            scope.bindings_dict[binding.name] = binding
            return
        if binding.kind == ANNOTATION_KIND_STR:
            return
        if is_submodule_pair_bool(existing, binding):
            scope.bindings_dict[binding.name] = binding
            return
        if existing.branch == binding.branch and not is_redirected_bool:
            self.check_redefinition_none(existing, binding)
        binding.is_used = binding.is_used or existing.is_used
        scope.bindings_dict[binding.name] = binding

    def check_outer_shadowing_none(self, binding: Binding) -> None:
        """F402 and F811 for a first local binding shadowing an import.

        Args:
            binding (Binding): New binding in a function or comprehension.
        Returns:
            None: F402 for loop variables shadowing any visible import;
            F811 when the directly enclosing (non-class) scope holds an
            unused import of the name.
        Warnings:
            None.
        """
        if self.scope.kind != FUNCTION_SCOPE_STR:
            return
        shadowed = self.find_binding_info(binding.name)
        if shadowed is None or shadowed.kind != IMPORT_KIND_STR:
            return
        if binding.kind == LOOP_KIND_STR:
            self.report("F402", binding.node,
                        f"Import `{binding.name}` from line "
                        f"{shadowed.node.lineno} shadowed by loop variable.")
            return
        if DUMMY_NAME_PATTERN.match(binding.name) or binding.kind in (
                ANNOTATION_KIND_STR, HANDLER_KIND_STR) or (
                is_submodule_pair_bool(shadowed, binding)) or (
                shadowed.is_typing_only):
            return
        self.redefinitions_list.append((shadowed, binding))

    def check_redefinition_none(self, existing: Binding,
                                binding: Binding) -> None:
        """F402 and F811 for a binding that replaces another.

        Args:
            existing (Binding): The binding being replaced.
            binding (Binding): The new binding.
        Returns:
            None: Reports at the new binding.
        Warnings:
            None.
        """
        if existing.kind == IMPORT_KIND_STR and binding.kind == LOOP_KIND_STR:
            self.report("F402", binding.node,
                        f"Import `{binding.name}` from line "
                        f"{existing.node.lineno} shadowed by loop variable.")
            return
        is_definition_bool = binding.kind in (DEFINITION_KIND_STR,
                                              CLASS_KIND_STR)
        if existing.is_used or not (
                existing.kind in REDEFINABLE_KINDS_FROZENSET or (
                    is_definition_bool
                    and existing.kind == ASSIGNMENT_KIND_STR)):
            return
        if DUMMY_NAME_PATTERN.match(binding.name) or (
                existing.is_typing_only and not binding.is_typing_only):
            return
        if binding.kind in (ANNOTATION_KIND_STR, DECLARATION_KIND_STR,
                            HANDLER_KIND_STR):
            return
        if is_overload_bool(existing.node) or is_property_pair_bool(
                existing.node, binding.node):
            return
        self.redefinitions_list.append((existing, binding))

    # ----- reads -----

    def find_binding_info(self, name_str: str) -> Binding | None:
        """Resolve a read through the scope chain.

        Args:
            name_str (str): The name.
        Returns:
            Binding | None: The binding, or None when nothing binds it.
        Warnings:
            None.
        """
        for index_int in range(len(self.stack_list) - 1, -1, -1):
            scope = self.stack_list[index_int]
            if scope.kind == CLASS_SCOPE_STR and index_int != len(
                    self.stack_list) - 1:
                continue
            if name_str in scope.bindings_dict:
                return scope.bindings_dict[name_str]
        return None

    def handle_load_none(self, node: ast.Name) -> None:
        """Resolve a name read and report it when undefined.

        Args:
            node (ast.Name): Load-context name.
        Returns:
            None: Marks the binding used, or reports F405, F821 or F823.
        Warnings:
            None.
        """
        name_str = node.id
        if name_str == "locals":
            self.scope.uses_locals = True
        if self.report_read_before_assignment_bool(node):
            return
        binding = self.find_binding_info(name_str)
        if binding is not None:
            mark_used_none(binding)
            return
        if name_str in BUILTIN_NAMES_FROZENSET or (
                name_str in CLASS_NAMES_FROZENSET
                and self.scope.kind == CLASS_SCOPE_STR) or (
                name_str == "__class__" and self.is_in_class_method_bool()):
            return
        stars_list = [module_str for scope in self.stack_list
                      for module_str in scope.star_modules_list]
        if stars_list:
            self.report("F405", node,
                        f"`{name_str}` may be undefined, or defined from "
                        f"star imports.")
            return
        if any("*" in names_set or name_str in names_set
               for names_set in self.handled_names_list):
            return
        self.report("F821", node, f"Undefined name `{name_str}`.")

    def report_read_before_assignment_bool(self, node: ast.Name) -> bool:
        """F823: a local name read before the function assigns it.

        Args:
            node (ast.Name): Load-context name.
        Returns:
            bool: True when F823 was reported (the name is assigned
                later in this function and also bound outside it).
        Warnings:
            None.
        """
        name_str, scope = node.id, self.scope
        if scope.kind == FUNCTION_SCOPE_STR and name_str in (
                scope.local_names_set) and name_str not in (
                scope.bindings_dict) and name_str not in scope.globals_set \
                and name_str not in scope.nonlocal_dict and (
                self.find_binding_info(name_str) is not None):
            self.report("F823", node,
                        f"Local variable `{name_str}` referenced before "
                        "assignment.")
            return True
        return False

    def is_in_class_method_bool(self) -> bool:
        """Whether the current function is nested in a class.

        Returns:
            bool: True inside a method (where __class__ exists).
        Warnings:
            None.
        """
        return any(scope.kind == CLASS_SCOPE_STR
                   for scope in self.stack_list[:-1])

    # ----- statements -----

    def visit_statements_none(self, statements_list: list[ast.stmt]) -> None:
        """Visit statements in order.

        Args:
            statements_list (list[ast.stmt]): A body.
        Returns:
            None.
        Warnings:
            None.
        """
        for statement in statements_list:
            self.visit_statement_none(statement)

    def visit_branches_none(self, node: ast.AST,
                            arms_list: list[list[ast.stmt]]) -> None:
        """Visit the arms of an if, try or match, each as its own branch.

        Args:
            node (ast.AST): The statement.
            arms_list (list[list[ast.stmt]]): Statement lists per arm.
        Returns:
            None.
        Warnings:
            None.
        """
        saved_tuple = self.branch_tuple
        for index_int, arm_list in enumerate(arms_list):
            self.branch_tuple = saved_tuple + ((id(node), index_int),)
            self.visit_statements_none(arm_list)
        self.branch_tuple = saved_tuple

    def visit_statement_none(self, node: ast.stmt) -> None:
        """Visit one statement.

        Args:
            node (ast.stmt): Statement.
        Returns:
            None.
        Warnings:
            None.
        """
        handler = getattr(self, "visit_" + type(node).__name__, None)
        if handler is not None:
            handler(node)
            return
        for child in ast.iter_child_nodes(node):
            if isinstance(child, ast.expr):
                self.visit_expression_none(child)
            elif isinstance(child, ast.stmt):
                self.visit_statement_none(child)

    def visit_FunctionDef(self, node: ast.AST) -> None:
        """Bind a function and defer its body.

        Args:
            node (ast.AST): Function definition.
        Returns:
            None.
        Warnings:
            None.
        """
        for expression in node.decorator_list:
            self.visit_expression_none(expression)
        arguments = node.args
        for expression in [*arguments.defaults, *arguments.kw_defaults]:
            if expression is not None:
                self.visit_expression_none(expression)
        type_scope = self.enter_type_scope_info(node)
        for argument in self.list_arguments_list(arguments):
            self.visit_annotation_none(argument.annotation)
        self.visit_annotation_none(node.returns)
        self.schedule_none(lambda: self.run_function_none(node))
        if type_scope is not None:
            self.stack_list.pop()
        self.add_binding_none(Binding(node.name, DEFINITION_KIND_STR, node))

    visit_AsyncFunctionDef = visit_FunctionDef

    def list_arguments_list(self, arguments: ast.arguments) -> list[ast.arg]:
        """Every parameter of a function or lambda.

        Args:
            arguments (ast.arguments): Parameters.
        Returns:
            list[ast.arg]: In declaration order.
        Warnings:
            None.
        """
        return [argument for argument in (
            *arguments.posonlyargs, *arguments.args, arguments.vararg,
            *arguments.kwonlyargs, arguments.kwarg) if argument is not None]

    def run_function_none(self, node: ast.AST) -> None:
        """Visit a deferred function or lambda body in its own scope.

        Args:
            node (ast.AST): Function definition or lambda.
        Returns:
            None: Defers the scope's unused checks after nested bodies.
        Warnings:
            None.
        """
        scope = Scope(FUNCTION_SCOPE_STR)
        scope.local_names_set = collect_local_names_set(node)
        self.stack_list.append(scope)
        for argument in self.list_arguments_list(node.args):
            self.add_binding_none(Binding(argument.arg, ARGUMENT_KIND_STR,
                                          argument))
        if isinstance(node, ast.Lambda):
            self.visit_expression_none(node.body)
        else:
            self.visit_statements_none(node.body)
        self.finished_scopes_list.append(scope)
        self.stack_list.pop()

    def enter_type_scope_info(self, node: ast.AST) -> Scope | None:
        """Enter a scope holding a generic definition's type parameters.

        Args:
            node (ast.AST): Function, class or type alias.
        Returns:
            Scope | None: The scope (the caller pops it), or None.
        Warnings:
            None.
        """
        if not getattr(node, "type_params", None):
            return None
        scope = Scope(TYPE_SCOPE_STR)
        self.stack_list.append(scope)
        for parameter in node.type_params:
            self.add_binding_none(Binding(parameter.name, ARGUMENT_KIND_STR,
                                          parameter))
        for parameter in node.type_params:
            for expression in (getattr(parameter, "bound", None),
                               getattr(parameter, "default_value", None)):
                if expression is not None:
                    self.visit_expression_none(expression)
        return scope

    def visit_ClassDef(self, node: ast.ClassDef) -> None:
        """Visit a class body now, then bind the class.

        Args:
            node (ast.ClassDef): Class definition.
        Returns:
            None.
        Warnings:
            None.
        """
        for expression in node.decorator_list:
            self.visit_expression_none(expression)
        type_scope = self.enter_type_scope_info(node)
        for expression in [*node.bases,
                           *(keyword.value for keyword in node.keywords)]:
            self.visit_expression_none(expression)
        scope = Scope(CLASS_SCOPE_STR)
        self.stack_list.append(scope)
        self.visit_statements_none(node.body)
        self.stack_list.pop()
        if type_scope is not None:
            self.stack_list.pop()
        self.add_binding_none(Binding(node.name, CLASS_KIND_STR, node))

    def visit_TypeAlias(self, node: ast.AST) -> None:
        """Bind a type alias; its value is evaluated lazily.

        Args:
            node (ast.AST): type statement.
        Returns:
            None.
        Warnings:
            None.
        """
        type_scope = self.enter_type_scope_info(node)
        self.schedule_none(lambda: self.visit_expression_none(node.value))
        if type_scope is not None:
            self.stack_list.pop()
        self.add_binding_none(Binding(node.name.id, ASSIGNMENT_KIND_STR,
                                      node.name))

    def visit_Assign(self, node: ast.Assign) -> None:
        """Visit the value, then bind the targets.

        Args:
            node (ast.Assign): Assignment.
        Returns:
            None.
        Warnings:
            None.
        """
        self.visit_expression_none(node.value)
        for target in node.targets:
            if len(node.targets) == 1 and isinstance(
                    target, (ast.Tuple, ast.List)) and isinstance(
                    node.value, (ast.Tuple, ast.List)) and len(
                    target.elts) == len(node.value.elts) and not any(
                    isinstance(element, ast.Starred)
                    for element in target.elts + node.value.elts):
                for element in target.elts:
                    self.bind_target_none(element, ASSIGNMENT_KIND_STR)
                continue
            self.bind_target_none(target, ASSIGNMENT_KIND_STR)
        self.collect_exports_none(node.targets, node.value)

    def visit_Expr(self, node: ast.Expr) -> None:
        """Visit an expression statement; __all__.extend/append export.

        Args:
            node (ast.Expr): Expression statement.
        Returns:
            None.
        Warnings:
            None.
        """
        self.visit_expression_none(node.value)
        call = node.value
        if isinstance(call, ast.Call) and isinstance(call.func,
                                                     ast.Attribute) and (
                isinstance(call.func.value, ast.Name)
                and call.func.value.id == "__all__" and call.args):
            argument = call.args[0]
            if call.func.attr == "append":
                argument = ast.List([argument], ast.Load(),
                                    lineno=argument.lineno,
                                    col_offset=argument.col_offset)
            if call.func.attr in ("append", "extend") and (
                    not self.branch_tuple):
                self.collect_exports_none([call.func.value], argument)

    def visit_AugAssign(self, node: ast.AugAssign) -> None:
        """Read and rebind the target of an augmented assignment.

        Args:
            node (ast.AugAssign): Augmented assignment.
        Returns:
            None.
        Warnings:
            None.
        """
        self.visit_expression_none(node.value)
        if isinstance(node.target, ast.Name):
            self.handle_load_none(ast.Name(node.target.id, ast.Load(),
                                           lineno=node.target.lineno,
                                           col_offset=node.target.col_offset))
            binding = Binding(node.target.id, ASSIGNMENT_KIND_STR,
                              node.target)
            binding.is_used = True
            self.add_binding_none(binding)
            self.collect_exports_none([node.target], node.value)
        else:
            self.visit_expression_none(node.target)

    def visit_AnnAssign(self, node: ast.AnnAssign) -> None:
        """Visit annotation and value, then bind the target.

        Args:
            node (ast.AnnAssign): Annotated assignment.
        Returns:
            None.
        Warnings:
            None.
        """
        self.visit_annotation_none(node.annotation)
        if node.value is not None:
            self.visit_expression_none(node.value)
        if not isinstance(node.target, ast.Name):
            self.visit_expression_none(node.target)
            return
        kind_str = (ASSIGNMENT_KIND_STR if node.value is not None
                    else ANNOTATION_KIND_STR)
        self.add_binding_none(Binding(node.target.id, kind_str, node.target))
        if node.value is not None:
            self.collect_exports_none([node.target], node.value)

    def visit_For(self, node: ast.AST) -> None:
        """Visit a for loop.

        Args:
            node (ast.AST): for or async for.
        Returns:
            None.
        Warnings:
            None.
        """
        self.visit_expression_none(node.iter)
        self.bind_target_none(node.target, LOOP_KIND_STR)
        self.visit_statements_none(node.body)
        self.visit_statements_none(node.orelse)

    visit_AsyncFor = visit_For

    def visit_If(self, node: ast.If) -> None:
        """Visit an if statement; its arms are separate branches.

        Args:
            node (ast.If): if statement.
        Returns:
            None.
        Warnings:
            None.
        """
        self.visit_expression_none(node.test)
        if is_type_checking_test_bool(node.test):
            saved_bool, self.in_type_checking_bool = (
                self.in_type_checking_bool, True)
            self.visit_branches_none(node, [node.body])
            self.in_type_checking_bool = saved_bool
            self.visit_branches_none(node, [[], node.orelse])
            return
        self.visit_branches_none(node, [node.body, node.orelse])

    def visit_With(self, node: ast.AST) -> None:
        """Visit with items, bind their targets, then the body.

        Args:
            node (ast.AST): with or async with.
        Returns:
            None.
        Warnings:
            None.
        """
        for with_item in node.items:
            self.visit_expression_none(with_item.context_expr)
            if with_item.optional_vars is not None:
                self.bind_target_none(with_item.optional_vars,
                                      ASSIGNMENT_KIND_STR)
        self.visit_statements_none(node.body)

    visit_AsyncWith = visit_With

    def visit_Try(self, node: ast.AST) -> None:
        """Visit try, handlers, else and finally.

        Args:
            node (ast.AST): try or try/except*.
        Returns:
            None.
        Warnings:
            None.
        """
        guards_set = {"*"} if any(
            is_name_error_handler_bool(handler)
            for handler in node.handlers) else set()
        self.handled_names_list.append(guards_set)
        self.try_ids_set.add(id(node))
        saved_tuple = self.branch_tuple
        self.branch_tuple = saved_tuple + ((id(node), 0),)
        self.visit_statements_none(node.body)
        self.handled_names_list.pop()
        for index_int, handler in enumerate(node.handlers, start=1):
            self.branch_tuple = saved_tuple + ((id(node), index_int),)
            self.visit_handler_none(handler)
        self.branch_tuple = saved_tuple + ((id(node), 0),)
        self.visit_statements_none(node.orelse)
        self.branch_tuple = saved_tuple
        self.visit_statements_none(node.finalbody)

    visit_TryStar = visit_Try

    def visit_handler_none(self, handler: ast.ExceptHandler) -> None:
        """Visit an except handler; its name is unbound afterwards.

        Args:
            handler (ast.ExceptHandler): Handler.
        Returns:
            None: Reports F841 when the name is never used.
        Warnings:
            None.
        """
        if handler.type is not None:
            self.visit_expression_none(handler.type)
        if handler.name is None:
            self.visit_statements_none(handler.body)
            return
        previous = self.scope.bindings_dict.get(handler.name)
        binding = Binding(handler.name, HANDLER_KIND_STR, handler)
        self.add_binding_none(binding)
        self.visit_statements_none(handler.body)
        if not binding.is_used:
            self.report("F841", handler,
                        f"Local variable `{handler.name}` is assigned to "
                        "but never used.")
        if self.scope.bindings_dict.get(handler.name) is binding:
            del self.scope.bindings_dict[handler.name]
            if previous is not None:
                self.scope.bindings_dict[handler.name] = previous

    def visit_Match(self, node: ast.Match) -> None:
        """Visit a match statement; each case is its own branch.

        Args:
            node (ast.Match): match statement.
        Returns:
            None.
        Warnings:
            None.
        """
        self.visit_expression_none(node.subject)
        saved_tuple = self.branch_tuple
        for index_int, case in enumerate(node.cases):
            self.branch_tuple = saved_tuple + ((id(node), index_int),)
            self.visit_pattern_none(case.pattern)
            if case.guard is not None:
                self.visit_expression_none(case.guard)
            self.visit_statements_none(case.body)
        self.branch_tuple = saved_tuple

    def visit_pattern_none(self, pattern: ast.AST) -> None:
        """Visit a match pattern: values are read, captures are bound.

        Args:
            pattern (ast.AST): Pattern.
        Returns:
            None.
        Warnings:
            None.
        """
        for node in ast.walk(pattern):
            if isinstance(node, ast.MatchValue):
                self.visit_expression_none(node.value)
            elif isinstance(node, ast.MatchClass):
                self.visit_expression_none(node.cls)
            elif isinstance(node, ast.MatchMapping):
                for key in node.keys:
                    self.visit_expression_none(key)
            name_str = getattr(node, "name", None) or getattr(
                node, "rest", None)
            if isinstance(name_str, str):
                self.add_binding_none(Binding(name_str, ASSIGNMENT_KIND_STR,
                                              node))

    def visit_Delete(self, node: ast.Delete) -> None:
        """Unbind deleted names.

        Args:
            node (ast.Delete): del statement.
        Returns:
            None: Reports F821 for names that were never bound.
        Warnings:
            None.
        """
        for target in node.targets:
            if not isinstance(target, ast.Name):
                self.visit_expression_none(target)
                continue
            scope = self.scope
            is_global_bool = target.id in scope.globals_set
            if is_global_bool:
                scope = self.module_scope
            if target.id in scope.bindings_dict:
                scope.bindings_dict[target.id].is_used = True
                if not self.branch_tuple and not is_global_bool:
                    del scope.bindings_dict[target.id]
            elif self.find_binding_info(target.id) is None and (
                    target.id not in BUILTIN_NAMES_FROZENSET):
                self.report("F821", target,
                            f"Undefined name `{target.id}`.")

    def visit_Global(self, node: ast.AST) -> None:
        """Declare names as module (or enclosing) names.

        Args:
            node (ast.AST): global or nonlocal statement.
        Returns:
            None.
        Warnings:
            None.
        """
        if isinstance(node, ast.Nonlocal):
            for name_str in node.names:
                for scope in reversed(self.stack_list[:-1]):
                    if scope.kind == FUNCTION_SCOPE_STR and (
                            name_str in scope.bindings_dict):
                        mark_used_none(scope.bindings_dict[name_str])
                        self.scope.nonlocal_dict[name_str] = scope
                        break
            return
        self.scope.globals_set.update(node.names)
        if isinstance(node, ast.Global):
            for name_str in node.names:
                if name_str not in self.module_scope.bindings_dict:
                    binding = Binding(name_str, DECLARATION_KIND_STR, node)
                    binding.is_used = True
                    self.module_scope.bindings_dict[name_str] = binding

    visit_Nonlocal = visit_Global

    def visit_Import(self, node: ast.Import) -> None:
        """Bind imported modules.

        Args:
            node (ast.Import): import statement.
        Returns:
            None.
        Warnings:
            None.
        """
        for alias in node.names:
            name_str = alias.asname or alias.name.split(".")[0]
            binding = Binding(name_str, IMPORT_KIND_STR, alias,
                              full_name=alias.name,
                              is_reexport=alias.asname == alias.name)
            binding.statement = node
            self.add_binding_none(binding)

    def visit_ImportFrom(self, node: ast.ImportFrom) -> None:
        """Bind imported names; record star imports.

        Args:
            node (ast.ImportFrom): from ... import statement.
        Returns:
            None: Reports F403 and F406 for star imports.
        Warnings:
            None.
        """
        module_str = "." * node.level + (node.module or "")
        for alias in node.names:
            if alias.name == "*":
                self.scope.star_modules_list.append(module_str)
                self.report("F403", node,
                            f"`from {module_str} import *` used; unable to "
                            "detect undefined names.")
                if self.scope.kind != MODULE_SCOPE_STR:
                    self.report("F406", node,
                                f"`from {module_str} import *` only allowed "
                                "at module level.")
                continue
            kind_str = (FUTURE_KIND_STR if module_str == "__future__"
                        else IMPORT_KIND_STR)
            binding = Binding(alias.asname or alias.name, kind_str, alias,
                              full_name=f"{module_str}.{alias.name}",
                              is_reexport=alias.asname == alias.name)
            binding.is_from_import = True
            binding.statement = node
            binding.is_used = kind_str == FUTURE_KIND_STR
            self.add_binding_none(binding)

    # ----- targets and expressions -----

    def bind_target_none(self, target: ast.AST, kind_str: str) -> None:
        """Bind the names of an assignment target.

        Args:
            target (ast.AST): Target expression.
            kind_str (str): Binding kind for plain names.
        Returns:
            None: Tuple and list targets bind unpacked names.
        Warnings:
            None.
        """
        if isinstance(target, ast.Name):
            self.add_binding_none(Binding(target.id, kind_str, target))
        elif isinstance(target, (ast.Tuple, ast.List)):
            element_kind_str = (UNPACKED_KIND_STR
                                if kind_str == ASSIGNMENT_KIND_STR
                                else kind_str)
            for element in target.elts:
                self.bind_target_none(element, element_kind_str)
        elif isinstance(target, ast.Starred):
            self.bind_target_none(target.value, kind_str)
        else:
            self.visit_expression_none(target)

    def visit_expression_none(self, expression: ast.AST) -> None:
        """Visit an expression's reads left to right, iteratively.

        Args:
            expression (ast.AST): Expression.
        Returns:
            None: Resolves reads; handles lambdas, comprehensions and
            assignment expressions.
        Warnings:
            None.
        """
        pending_list = [expression]
        while pending_list:
            node = pending_list.pop()
            if isinstance(node, ast.Name):
                if isinstance(node.ctx, ast.Load):
                    self.handle_load_none(node)
                elif isinstance(node.ctx, ast.Store):
                    self.add_binding_none(Binding(
                        node.id, ASSIGNMENT_KIND_STR, node))
            elif isinstance(node, ast.Lambda):
                for default in [*node.args.defaults, *node.args.kw_defaults]:
                    if default is not None:
                        self.visit_expression_none(default)
                self.schedule_none(lambda node=node: self.run_function_none(
                    node))
            elif isinstance(node, COMPREHENSION_NODES_TUPLE):
                self.visit_comprehension_none(node)
            elif isinstance(node, ast.Call) and self.visit_typing_call_bool(
                    node):
                continue
            elif isinstance(node, ast.NamedExpr):
                self.visit_expression_none(node.value)
                self.bind_walrus_none(node.target)
            else:
                pending_list.extend(reversed(list(
                    ast.iter_child_nodes(node))))

    def visit_comprehension_none(self, node: ast.AST) -> None:
        """Visit a comprehension in its own scope.

        Args:
            node (ast.AST): List, set or dict comprehension, or generator.
        Returns:
            None: The first iterable is read in the enclosing scope.
        Warnings:
            None.
        """
        self.visit_expression_none(node.generators[0].iter)
        self.stack_list.append(Scope(COMPREHENSION_SCOPE_STR))
        for index_int, generator in enumerate(node.generators):
            if index_int:
                self.visit_expression_none(generator.iter)
            self.bind_target_none(generator.target, ASSIGNMENT_KIND_STR)
            for condition in generator.ifs:
                self.visit_expression_none(condition)
        for part in ((node.key, node.value) if isinstance(node, ast.DictComp)
                     else (node.elt,)):
            self.visit_expression_none(part)
        self.stack_list.pop()

    def bind_walrus_none(self, target: ast.Name) -> None:
        """Bind an assignment expression in the nearest non-comprehension.

        Args:
            target (ast.Name): The walrus target.
        Returns:
            None.
        Warnings:
            None.
        """
        index_int = len(self.stack_list) - 1
        while index_int > 0 and self.stack_list[index_int].kind == (
                COMPREHENSION_SCOPE_STR):
            index_int -= 1
        saved_list = self.stack_list
        self.stack_list = saved_list[:index_int + 1]
        self.add_binding_none(Binding(target.id, ASSIGNMENT_KIND_STR, target))
        self.stack_list = saved_list

    def visit_annotation_none(self, annotation: ast.AST | None) -> None:
        """Visit an annotation, deferring postponed and string ones.

        Args:
            annotation (ast.AST | None): Annotation expression.
        Returns:
            None.
        Warnings:
            None.
        """
        if annotation is None:
            return
        if self.future_annotations_bool:
            self.schedule_none(lambda: self.visit_type_expression_none(
                annotation))
        else:
            self.visit_type_expression_none(annotation)

    def visit_type_expression_none(self, annotation: ast.AST) -> None:
        """Read a type expression; strings are parsed and read later.

        Args:
            annotation (ast.AST): Type expression.
        Returns:
            None: Literal[...] contents and Annotated metadata are not
            types and are read as plain values.
        Warnings:
            None.
        """
        pending_list = [annotation]
        while pending_list:
            node = pending_list.pop()
            if isinstance(node, ast.Constant) and isinstance(node.value, str):
                parsed = parse_string_annotation_info(node)
                if parsed is not None:
                    self.schedule_none(lambda parsed=parsed: (
                        self.visit_type_expression_none(parsed)))
            elif isinstance(node, ast.Subscript) and self.is_typing_name_bool(
                    node.value, "Literal"):
                self.visit_expression_none(node.value)
            elif isinstance(node, ast.Subscript) and self.is_typing_name_bool(
                    node.value, "Annotated") and isinstance(node.slice,
                                                            ast.Tuple):
                self.visit_expression_none(node.value)
                pending_list.append(node.slice.elts[0])
                for metadata in node.slice.elts[1:]:
                    self.visit_expression_none(metadata)
            elif isinstance(node, ast.Name):
                self.handle_load_none(node)
            else:
                pending_list.extend(reversed(list(
                    ast.iter_child_nodes(node))))

    def visit_typing_call_bool(self, node: ast.Call) -> bool:
        """Visit cast() and TypeVar() calls, whose strings are types.

        Args:
            node (ast.Call): A call.
        Returns:
            bool: True when the call was handled here.
        Warnings:
            None.
        """
        if self.is_typing_name_bool(node.func, "cast") and node.args:
            self.visit_expression_none(node.func)
            self.visit_type_expression_none(node.args[0])
            for argument in node.args[1:]:
                self.visit_expression_none(argument)
            return True
        if self.is_typing_name_bool(node.func, "TypeVar"):
            self.visit_expression_none(node.func)
            for argument in node.args[1:]:
                self.visit_type_expression_none(argument)
            for keyword in node.keywords:
                if keyword.arg == "bound":
                    self.visit_type_expression_none(keyword.value)
                else:
                    self.visit_expression_none(keyword.value)
            return True
        return False

    def is_typing_name_bool(self, node: ast.AST, name_str: str) -> bool:
        """Whether an expression refers to typing's (or typing_extensions')
        special form of that name.

        Args:
            node (ast.AST): The subscripted value.
            name_str (str): "Literal" or "Annotated".
        Returns:
            bool: True for an imported typing.X or typing_extensions.X,
            or for typing.X / t.X attribute access on an imported module.
        Warnings:
            None.
        """
        if isinstance(node, ast.Attribute):
            if node.attr != name_str or not isinstance(node.value, ast.Name):
                return False
            binding = self.find_binding_info(node.value.id)
            return binding is not None and binding.full_name in (
                "typing", "typing_extensions")
        if not isinstance(node, ast.Name):
            return False
        binding = self.find_binding_info(node.id)
        return binding is not None and binding.full_name in (
            f"typing.{name_str}", f"typing_extensions.{name_str}")

    # ----- exports and unused bindings -----

    def collect_exports_none(self, targets_list: list[ast.AST],
                             value_node: ast.AST) -> None:
        """Record names listed in a module-level __all__.

        Args:
            targets_list (list[ast.AST]): Assignment targets.
            value_node (ast.AST): Assigned value.
        Returns:
            None.
        Warnings:
            None.
        """
        if self.scope is not self.module_scope or not any(
                isinstance(target, ast.Name) and target.id == "__all__"
                for target in targets_list):
            return
        pending_list = [value_node]
        while pending_list:
            part = pending_list.pop()
            if isinstance(part, ast.BinOp) and isinstance(part.op, ast.Add):
                pending_list.extend((part.left, part.right))
            elif isinstance(part, (ast.List, ast.Tuple)):
                for element in part.elts:
                    if isinstance(element, ast.Constant) and isinstance(
                            element.value, str):
                        self.all_names_list.append((element.value, element,
                                                    value_node))

    def check_exports_none(self) -> None:
        """F822 for undefined names in __all__; exported imports are used.

        Returns:
            None.
        Warnings:
            None.
        """
        for name_str, node, statement in self.all_names_list:
            binding = self.module_scope.bindings_dict.get(name_str)
            if binding is not None:
                mark_used_none(binding)
            elif name_str in BUILTIN_NAMES_FROZENSET:
                continue
            elif self.module_scope.star_modules_list:
                self.report("F405", node,
                            f"`{name_str}` may be undefined, or defined "
                            "from star imports.", statement)
            elif not self.is_submodule_bool(name_str):
                self.report("F822", node,
                            f"Undefined name `{name_str}` in `__all__`.",
                            statement)

    def is_submodule_bool(self, name_str: str) -> bool:
        """Whether a package __init__ exports a submodule of that name.

        Args:
            name_str (str): Name listed in __all__.
        Returns:
            bool: True when this file is an __init__.py and name.py or a
            name/ folder sits next to it.
        Warnings:
            Only the file system is consulted; nothing is imported.
        """
        if not self.is_init_bool or self.folder is None:
            return False
        return (self.folder / f"{name_str}.py").is_file() or (
            self.folder / name_str).is_dir()

    def check_unused_imports_none(self, scope: Scope) -> None:
        """F401 for imports never read in a scope.

        Args:
            scope (Scope): The finished scope.
        Returns:
            None.
        Warnings:
            None.
        """
        used_roots_set = {binding.full_name.split(".")[0]
                          for binding in scope.history_list
                          if binding.kind == IMPORT_KIND_STR
                          and binding.is_used and binding.node.asname
                          and not binding.is_from_import}
        imports_list = [binding for binding in scope.history_list
                        if binding.kind == IMPORT_KIND_STR
                        and is_submodule_import_bool(binding)]
        for binding in [*scope.bindings_dict.values(), *[
                member for member in imports_list
                if scope.bindings_dict.get(member.name) is not member]]:
            if binding.kind != IMPORT_KIND_STR or binding.is_used:
                continue
            if "." in binding.full_name and not binding.full_name.startswith(
                    ".") and binding.name == binding.full_name.split(".")[0] \
                    and binding.name in used_roots_set:
                continue
            if not binding.is_reexport:
                self.report("F401", binding.node,
                            f"`{binding.full_name.lstrip('.')}` imported "
                            "but unused.", binding.statement)

    def check_function_scope_none(self, scope: Scope) -> None:
        """F401, F841 and F842 when a function scope has finished.

        Args:
            scope (Scope): The function's scope.
        Returns:
            None.
        Warnings:
            None.
        """
        self.check_unused_imports_none(scope)
        if scope.uses_locals:
            return
        for name_str, binding in scope.bindings_dict.items():
            if binding.is_used or name_str in scope.globals_set or (
                    DUMMY_NAME_PATTERN.match(name_str)) or (
                    name_str in TRACEBACK_NAMES_FROZENSET):
                continue
            if binding.kind == ASSIGNMENT_KIND_STR:
                self.report("F841", binding.node,
                            f"Local variable `{name_str}` is assigned to "
                            "but never used.")
            elif binding.kind == ANNOTATION_KIND_STR:
                self.report("F842", binding.node,
                            f"Local variable `{name_str}` is annotated but "
                            "never used.")


def is_overload_bool(node: ast.AST) -> bool:
    """Whether a definition is decorated with typing.overload.

    Args:
        node (ast.AST): A binding node.
    Returns:
        bool: True for @overload / @typing.overload functions.
    Warnings:
        None.
    """
    if not isinstance(node, FUNCTION_NODES_TUPLE):
        return False
    return any((isinstance(decorator, ast.Name) and decorator.id == "overload")
               or (isinstance(decorator, ast.Attribute)
                   and decorator.attr == "overload")
               for decorator in node.decorator_list)


def is_property_pair_bool(existing_node: ast.AST, node: ast.AST) -> bool:
    """Whether a function redefines a property through @name.setter etc.

    Args:
        existing_node (ast.AST): The earlier definition.
        node (ast.AST): The new definition.
    Returns:
        bool: True for property setter, getter and deleter chains.
    Warnings:
        None.
    """
    if not isinstance(node, FUNCTION_NODES_TUPLE):
        return False
    return any(isinstance(decorator, ast.Attribute)
               and isinstance(decorator.value, ast.Name)
               and decorator.value.id == node.name
               for decorator in node.decorator_list)


def is_name_error_handler_bool(handler: ast.ExceptHandler) -> bool:
    """Whether an except handler catches NameError.

    Args:
        handler (ast.ExceptHandler): Handler.
    Returns:
        bool: True for except NameError and tuples containing it.
    Warnings:
        None.
    """
    caught = handler.type
    names_list = caught.elts if isinstance(caught, ast.Tuple) else [caught]
    return any(isinstance(name, ast.Name) and name.id == "NameError"
               for name in names_list)


def get_subscript_name_str(node: ast.Subscript) -> str:
    """The last name part of a subscripted expression.

    Args:
        node (ast.Subscript): Subscript.
    Returns:
        str: For example "Literal" for typing.Literal[...].
    Warnings:
        None.
    """
    subscripted = node.value
    if isinstance(subscripted, ast.Attribute):
        return subscripted.attr
    return subscripted.id if isinstance(subscripted, ast.Name) else ""


def parse_string_annotation_info(node: ast.Constant) -> ast.AST | None:
    """Parse a string annotation, positioned inside the string.

    Args:
        node (ast.Constant): String constant.
    Returns:
        ast.AST | None: The parsed expression with locations shifted to
        the string's position, or None when it is not valid.
    Warnings:
        None.
    """
    try:
        parsed = ast.parse(node.value.strip(), mode="eval").body
    except SyntaxError:
        return None
    for child in ast.walk(parsed):
        if hasattr(child, "lineno"):
            child.lineno = node.lineno
            child.col_offset = node.col_offset + 1 + child.col_offset
            child.end_lineno = node.lineno
    return parsed



def locate_report_tuple(context_info: RuleContext,
                        node: ast.AST) -> tuple[int, int]:
    """Where a scope finding is reported.

    Args:
        context_info (RuleContext): Source access.
        node (ast.AST): The binding or read node.
    Returns:
        tuple[int, int]: The definition name for def/class, the name
        after "as" for except handlers, else the node's start.
    Warnings:
        None.
    """
    if isinstance(node, (*FUNCTION_NODES_TUPLE, ast.ClassDef)):
        return locate_definition_name_tuple(context_info.source_info, node)
    if isinstance(node, ast.ExceptHandler) and node.name:
        return find_identifier_tuple(context_info, node, node.name)
    line_int, column_int = locate_node_tuple(context_info.source_info, node)
    if isinstance(node, ast.alias) and node.asname:
        line_str = context_info.source_info.lines[line_int - 1]
        match_info = re.compile(r"\bas\s+" + re.escape(node.asname)
                                + r"\b").search(line_str, column_int - 1)
        if match_info:
            return line_int, match_info.end() - len(node.asname) + 1
    return line_int, column_int


def build_scope_reporter(context_info: RuleContext) -> Callable[..., None]:
    """The report callback the scope checker uses for one file.

    Args:
        context_info (RuleContext): Parsed file, settings and findings.
    Returns:
        Callable[..., None]: report(code, node, message, parent=None).
    Warnings:
        None.
    """

    def report_finding_none(code_str: str, node: ast.AST, message_str: str,
                            parent: ast.AST | None = None) -> None:
        """Report a finding at the node, with its statement for noqa.

        Args:
            code_str (str): Rule code.
            node (ast.AST): Node to report at.
            message_str (str): Explanation.
            parent (ast.AST | None): Statement whose first line may hold
                the noqa comment.
        Returns:
            None: Adds the finding.
        Warnings:
            None.
        """
        context_info.report_none(
            code_str, locate_report_tuple(context_info, node), message_str,
            parent.lineno if parent is not None else 0)

    return report_finding_none


def check_scope_codes_none(context_info: RuleContext) -> None:
    """Run the scope checker when any scope code is selected.

    Args:
        context_info (RuleContext): Parsed file, settings and findings.
    Returns:
        None: Adds F4xx, F8xx scope findings.
    Warnings:
        Files nested beyond the interpreter's recursion limit are skipped.
    """
    if not any(context_info.is_enabled_bool(code_str)
               for code_str in SCOPE_CODES_TUPLE):
        return
    report_finding_none = build_scope_reporter(context_info)
    is_init_bool = Path(context_info.source_info.path).name == "__init__.py"
    limit_int = sys.getrecursionlimit()
    sys.setrecursionlimit(max(limit_int, 20000))
    try:
        ScopeChecker(context_info.tree, report_finding_none, is_init_bool,
                     Path(context_info.source_info.path).parent).run_none()
    except RecursionError:
        pass
    finally:
        sys.setrecursionlimit(limit_int)


def mark_used_none(binding: Binding) -> None:
    """Mark a binding used; a read of a package name also uses every
    "import package.sub" made so far in that scope.

    Args:
        binding (Binding): The binding a read resolved to.
    Returns:
        None.
    Warnings:
        None.
    """
    binding.is_used = True
    if binding.kind != IMPORT_KIND_STR or binding.scope is None:
        return
    for member in binding.scope.history_list:
        if member.kind == IMPORT_KIND_STR and member.name == binding.name and (
                is_submodule_import_bool(member)
                or is_submodule_import_bool(binding)):
            member.is_used = True


def is_submodule_import_bool(binding: Binding) -> bool:
    """Whether a binding is a plain "import a.b" (binding the name a).

    Args:
        binding (Binding): An import binding.
    Returns:
        bool: True for dotted imports without an alias.
    Warnings:
        None.
    """
    return not binding.is_from_import and "." in binding.full_name and (
        binding.name == binding.full_name.split(".")[0])


def is_submodule_pair_bool(existing: Binding, binding: Binding) -> bool:
    """Whether two imports bind one name for a package and a submodule.

    Args:
        existing (Binding): Earlier binding.
        binding (Binding): New binding.
    Returns:
        bool: True for "import a" with "import a.b" (either order), which
        share one usage.
    Warnings:
        None.
    """
    return (existing.kind == IMPORT_KIND_STR == binding.kind
            and existing.full_name != binding.full_name
            and "." in existing.full_name + binding.full_name
            and existing.full_name.split(".")[0] == binding.full_name.split(
                ".")[0] == binding.name)


def collect_local_names_set(node: ast.AST) -> set[str]:
    """Names a function body assigns (its locals), nested scopes aside.

    Args:
        node (ast.AST): Function definition or lambda.
    Returns:
        set[str]: Assigned, imported, defined and deleted names, minus
        those declared global or nonlocal.
    Warnings:
        None.
    """
    names_set: set[str] = set()
    declared_set: set[str] = set()
    pending_list = list(node.body) if isinstance(node.body, list) else [
        node.body]
    while pending_list:
        child = pending_list.pop()
        if isinstance(child, (ast.Global, ast.Nonlocal)):
            declared_set.update(child.names)
        elif isinstance(child, ast.Name) and not isinstance(child.ctx,
                                                           ast.Load):
            names_set.add(child.id)
        elif isinstance(child, (*FUNCTION_NODES_TUPLE, ast.ClassDef)):
            names_set.add(child.name)
            pending_list.extend(child.decorator_list)
            continue
        elif isinstance(child, ast.alias):
            names_set.add(child.asname or child.name.split(".")[0])
        elif isinstance(child, (ast.Lambda, *COMPREHENSION_NODES_TUPLE)):
            continue
        pending_list.extend(ast.iter_child_nodes(child))
    return names_set - declared_set


def is_type_checking_test_bool(test: ast.AST) -> bool:
    """Whether an if test is TYPE_CHECKING or typing.TYPE_CHECKING.

    Args:
        test (ast.AST): The if test.
    Returns:
        bool: True for the typing-only guard.
    Warnings:
        None.
    """
    return (isinstance(test, ast.Name) and test.id == "TYPE_CHECKING") or (
        isinstance(test, ast.Attribute) and test.attr == "TYPE_CHECKING")
