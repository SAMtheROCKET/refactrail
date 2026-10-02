"""The per-file context every rule receives, with lazily built facts."""

from functools import cached_property
import ast

from refactrail.facts import (
    FunctionFact, collect_function_facts_list, find_constant_candidates_dict,
    find_type_alias_names_set,
)
from refactrail.models import (
    ERROR_SEVERITY_STR, WARNING_SEVERITY_STR, Finding, Settings,
    is_code_enabled_bool,
)
from refactrail.scopes import Binding, BindingCollector
from refactrail.source import SourceFile
from refactrail.wordlists import load_word_set

ERROR_CODES_TUPLE = ("RT000", "RT001", "RT002", "RT101", "RT502",
                     "RT503")
VERBS_SET = load_word_set("verbs.txt")
GENERIC_NAMES_SET = load_word_set("generic_names.txt")
PROTOCOL_HOOKS_SET = load_word_set("protocol_hooks.txt")


def is_protocol_hook_bool(name_str: str) -> bool:
    """Tell whether a method name is required by a framework.

    Args:
        name_str (str): Method name.
    Returns:
        bool: True for names such as setUp or visit_Name.
    Warnings:
        A trailing * in the list matches any suffix.
    """
    return any(name_str == hook_str or (hook_str.endswith("*")
                                        and name_str.startswith(hook_str[:-1]))
               for hook_str in PROTOCOL_HOOKS_SET)


class RuleContext:
    """Give rules the parsed file, settings and shared facts.

    Args:
        source_info (SourceFile): Decoded file.
        tree (ast.Module): Parsed module.
        settings_info (Settings): Active settings.
    Returns:
        RuleContext: Rules call report_none() to add findings.
    Warnings:
        Facts are computed once, on first use.
    """

    def __init__(self, source_info: SourceFile, tree: ast.Module,
                 settings_info: Settings) -> None:
        """Store the inputs and start with no findings.

        Args:
            source_info (SourceFile): Decoded file.
            tree (ast.Module): Parsed module.
            settings_info (Settings): Active settings.
        Returns:
            None: Initialises the context.
        Warnings:
            None.
        """
        self.source_info = source_info
        self.tree = tree
        self.settings_info = settings_info
        self.findings_list: list[Finding] = []

    @cached_property
    def bindings_list(self) -> list[Binding]:
        """Return the earliest binding of each name per scope.

        Args:
            None: Uses the parsed tree.
        Returns:
            list[Binding]: Bindings in position order.
        Warnings:
            Computed once per file.
        """
        return BindingCollector(self.source_info).collect_bindings_list(
            self.tree)

    @cached_property
    def constants_dict(self) -> dict[str, ast.stmt]:
        """Return module-level constant candidates by name.

        Args:
            None: Uses the parsed tree.
        Returns:
            dict[str, ast.stmt]: Name to assignment statement.
        Warnings:
            Computed once per file.
        """
        return find_constant_candidates_dict(self.tree)

    @cached_property
    def type_aliases_set(self) -> set[str]:
        """Return the names of module-level type aliases.

        Args:
            None: Uses the parsed tree.
        Returns:
            set[str]: Alias names (RT103, RT204).
        Warnings:
            Computed once per file.
        """
        return find_type_alias_names_set(self.tree)

    @cached_property
    def functions_list(self) -> list[FunctionFact]:
        """Return every function with its method status.

        Args:
            None: Uses the parsed tree.
        Returns:
            list[FunctionFact]: Functions in source order.
        Warnings:
            Computed once per file.
        """
        return collect_function_facts_list(self.tree)

    def is_enabled_bool(self, code_str: str) -> bool:
        """Tell whether a rule should run in this file.

        Args:
            code_str (str): Rule code.
        Returns:
            bool: True when selected and allowed by the profile.
        Warnings:
            Rules may skip expensive work when this is False.
        """
        return is_code_enabled_bool(self.settings_info, code_str)

    def report_none(self, code_str: str, position_tuple: tuple[int, int],
                    message_str: str) -> None:
        """Add a finding unless disabled or suppressed with # noqa.

        Args:
            code_str (str): Rule code.
            position_tuple (tuple[int, int]): Line and column.
            message_str (str): Explanation.
        Returns:
            None: Appends to findings_list.
        Warnings:
            # noqa applies to the line where the finding is reported.
        """
        if not self.is_enabled_bool(code_str):
            return
        line_int, column_int = position_tuple
        suppressed = self.source_info.noqa.get(line_int, frozenset())
        if suppressed is None or any(code_str.startswith(prefix_str)
                                     for prefix_str in suppressed):
            return
        severity_str = (ERROR_SEVERITY_STR if code_str in ERROR_CODES_TUPLE
                        else WARNING_SEVERITY_STR)
        self.findings_list.append(Finding(
            self.source_info.path, line_int, column_int, code_str,
            severity_str, message_str))
