"""Size rules for functions and the main block."""

from refactrail.facts import is_main_block_bool
from refactrail.rules.context import RuleContext
from refactrail.source import locate_definition_name_tuple, locate_node_tuple


def check_function_sizes_none(context: RuleContext) -> None:
    """Report functions over the preferred or maximum span (RT501/RT502).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds one finding per long function.
    Warnings:
        The span starts at the first decorator.
    """
    settings_info = context.settings_info
    for fact in context.functions_list:
        node = fact.node
        first_int = min([node.lineno] + [
            decorator_node.lineno for decorator_node in node.decorator_list])
        span_int = node.end_lineno - first_int + 1
        position_tuple = locate_definition_name_tuple(context.source_info,
                                                      node)
        if span_int > settings_info.function_max_lines:
            context.report_none("RT502", position_tuple, (
                f"Function '{node.name}' spans {span_int} lines; limit "
                f"{settings_info.function_max_lines}."))
        elif span_int > settings_info.function_preferred_lines:
            context.report_none("RT501", position_tuple, (
                f"Function '{node.name}' spans {span_int} lines; preferred "
                f"{settings_info.function_preferred_lines}."))


def check_main_block_size_none(context: RuleContext) -> None:
    """Report a main block longer than its limit (RT503).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds one finding per long main block.
    Warnings:
        Only top-level if __name__ == "__main__": blocks are measured.
    """
    limit_int = context.settings_info.main_max_lines
    for node in context.tree.body:
        if not is_main_block_bool(node):
            continue
        span_int = node.end_lineno - node.lineno + 1
        if span_int > limit_int:
            context.report_none("RT503", locate_node_tuple(
                context.source_info, node), (
                f"The main block spans {span_int} lines; limit "
                f"{limit_int}."))
