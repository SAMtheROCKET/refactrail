"""Every rule of the Python reference engine, in execution order."""

from refactrail.rules.annotations import (
    check_parameter_annotations_none, check_return_annotations_none,
)
from refactrail.rules.docs import (
    check_docstring_sections_none, check_documented_arguments_none,
    check_missing_docstrings_none,
)
from refactrail.rules.layout import (
    check_constant_names_none, check_constant_placement_none,
    check_line_length_none, check_module_code_none,
)
from refactrail.rules.naming import (
    check_conventions_none, check_dtype_suffixes_none,
    check_generic_names_none, check_short_names_none, check_verb_names_none,
)
from refactrail.rules.signatures import check_signature_suffixes_none
from refactrail.rules.size import (
    check_function_sizes_none, check_main_block_size_none,
)

RULE_CHECKS_TUPLE = (
    check_line_length_none, check_constant_names_none,
    check_constant_placement_none, check_module_code_none,
    check_short_names_none, check_verb_names_none, check_dtype_suffixes_none,
    check_conventions_none, check_generic_names_none,
    check_signature_suffixes_none,
    check_missing_docstrings_none, check_docstring_sections_none,
    check_documented_arguments_none, check_parameter_annotations_none,
    check_return_annotations_none, check_function_sizes_none,
    check_main_block_size_none,
)
RULE_TITLES_DICT = {
    "RT001": "Syntax error", "RT002": "Unsupported encoding",
    "RT101": "Line too long", "RT102": "Constant not UPPER_CASE",
    "RT103": "Constant after other code",
    "RT201": "Single-character name",
    "RT202": "Function name is not a verb",
    "RT203": "Missing dtype suffix (strict)",
    "RT204": "Naming convention", "RT205": "Generic name (strict)",
    "RT206": "Return dtype suffix (strict)",
    "RT207": "Parameter dtype suffix (strict)",
    "RT301": "Missing docstring", "RT302": "Incomplete docstring",
    "RT303": "Docstring arguments mismatch",
    "RT401": "Parameter annotation missing",
    "RT402": "Return annotation missing",
    "RT501": "Function over preferred length",
    "RT502": "Function over length limit",
    "RT503": "Main block too long", "RT504": "Code outside functions",
}
