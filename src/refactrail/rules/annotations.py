"""Type-annotation rules for parameters and return values."""

from refactrail.facts import read_decorator_names_list
from refactrail.rules.context import RuleContext
from refactrail.source import (
    locate_definition_name_tuple, locate_node_tuple,
)


def check_parameter_annotations_none(context: RuleContext) -> None:
    """Report parameters without type annotations (RT401).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        The implicit self/cls parameter of a method is exempt.
    """
    for fact in context.functions_list:
        node = fact.node
        arguments_node = node.args
        positional_list = [*arguments_node.posonlyargs, *arguments_node.args]
        implicit_node = (positional_list[0] if fact.is_method
                         and positional_list
                         and "staticmethod"
                         not in read_decorator_names_list(node) else None)
        for parameter_node in filter(None, [
            *positional_list, arguments_node.vararg,
            *arguments_node.kwonlyargs, arguments_node.kwarg,
        ]):
            if parameter_node is implicit_node or (
                parameter_node.annotation is not None
            ):
                continue
            context.report_none("RT401", locate_node_tuple(
                context.source_info, parameter_node), (
                f"Parameter '{parameter_node.arg}' of '{node.name}' has "
                "no type annotation."))


def check_return_annotations_none(context: RuleContext) -> None:
    """Report functions without a return annotation (RT402).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        __init__ is exempt in the standard profile only.
    """
    strict_bool = context.settings_info.strict_bool
    for fact in context.functions_list:
        node = fact.node
        if node.returns is not None or (
            node.name == "__init__" and not strict_bool
        ):
            continue
        context.report_none("RT402", locate_definition_name_tuple(
            context.source_info, node),
            f"Function '{node.name}' has no return annotation.")
