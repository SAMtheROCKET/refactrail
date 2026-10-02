"""Notebook formatting preserves outputs, metadata and unrelated JSON bytes."""

from copy import deepcopy
import json
import re

from refactrail.json_spans import collect_json_spans_dict


def build_unique_dict(pairs_list: list[tuple]) -> dict:
    """Reject duplicate JSON keys before proposing container edits.

    Args:
        pairs_list (list[tuple]): Decoded object members in source order.
    Returns:
        dict: Unique object mapping.
    Warnings:
        Ambiguous duplicate members are never silently discarded.
    """
    mapping_dict = dict(pairs_list)
    if len(mapping_dict) != len(pairs_list):
        raise ValueError("Notebook contains duplicate JSON keys")
    return mapping_dict


def read_notebook_dict(text_str: str) -> dict:
    """Validate the Python notebook container and source representations.

    Args:
        text_str (str): UTF-8 decoded notebook JSON.
    Returns:
        dict: Validated notebook version four.
    Warnings:
        Cell magics are not executed or stripped.
    """
    notebook_dict = json.loads(
        text_str, object_pairs_hook=build_unique_dict,
        parse_constant=raise_json_constant_none)
    if not isinstance(notebook_dict, dict) or (
        type(notebook_dict.get("nbformat")) is not int
        or notebook_dict["nbformat"] != 4
    ):
        raise ValueError("Expected a notebook with nbformat 4")
    metadata_dict = notebook_dict.get("metadata", {})
    if not isinstance(metadata_dict, dict):
        raise ValueError("Notebook metadata must be an object")
    for key_str, field_str in (("language_info", "name"),
                              ("kernelspec", "language")):
        entry_dict = metadata_dict.get(key_str, {})
        language_str = (entry_dict.get(field_str, "python")
                        if isinstance(entry_dict, dict) else None)
        if not isinstance(language_str, str) or language_str.lower() not in (
            "python", "python3"
        ):
            raise ValueError("Only Python notebooks are supported")
    if not isinstance(notebook_dict.get("cells"), list):
        raise ValueError("Notebook cells must be an array")
    for index_int, cell_dict in enumerate(notebook_dict["cells"]):
        if not isinstance(cell_dict, dict) or (
            cell_dict.get("cell_type") not in ("code", "markdown", "raw")
        ):
            raise ValueError(f"Invalid notebook cell {index_int + 1}")
        read_cell_source_str(cell_dict, index_int)
    return notebook_dict


def read_cell_source_str(cell_dict: dict, index_int: int) -> str:
    """Read a cell source without interpreting notebook commands.

    Args:
        cell_dict (dict): Validated cell object.
        index_int (int): Zero-based cell index.
    Returns:
        str: Joined source text.
    Warnings:
        Non-string array members are refused rather than coerced.
    """
    source_value = cell_dict.get("source")
    if isinstance(source_value, str):
        return source_value
    if isinstance(source_value, list) and all(isinstance(line_str, str)
                                              for line_str in source_value):
        return "".join(source_value)
    raise ValueError(
        f"Cell {index_int + 1} source must be a string or string array")


def format_cell_str(source_str: str, label_str: str, width_int: int | None,
                    hug_bool: bool) -> str:
    """Format one code cell, naming the cell in any refusal.

    Args:
        source_str (str): Cell source.
        label_str (str): Notebook path with the cell number.
        width_int (int | None): Optional bounded wrapping width.
        hug_bool (bool): Keep closing brackets on the last content line.
    Returns:
        str: Validated cell source.
    Warnings:
        Raises ValueError naming the cell when it cannot be formatted.
    """
    from refactrail.formatting import format_source_str
    try:
        return format_source_str(source_str, label_str, width_int, hug_bool)
    except (ValueError, SyntaxError) as error:
        raise ValueError(f"Cannot format {label_str}: {error}") from error


def format_notebook_str(text_str: str, path_str: str,
                         width_int: int | None = None,
                         hug_bool: bool = False) -> str:
    """Format Python cells with replacements confined to source JSON values.

    Args:
        text_str (str): Original notebook JSON.
        path_str (str): Source label for cell-specific refusals.
        width_int (int | None): Optional bounded wrapping width.
        hug_bool (bool): Keep closing brackets on the last content line.
    Returns:
        str: Notebook preserving every unrelated original character.
    Warnings:
        Unsupported syntax in any code cell refuses the entire notebook.
    """
    notebook_dict = read_notebook_dict(text_str)
    spans_dict = collect_json_spans_dict(text_str)
    edits_list = []
    for index_int, cell_dict in enumerate(notebook_dict["cells"]):
        if cell_dict["cell_type"] != "code":
            continue
        source_str = read_cell_source_str(cell_dict, index_int)
        output_str = format_cell_str(source_str, f"{path_str}#cell="
                                     f"{index_int + 1}", width_int, hug_bool)
        if output_str == source_str:
            continue
        source_value = output_str
        if isinstance(cell_dict["source"], list):
            source_value = re.findall(
                r"[^\r\n]*(?:\r\n|\r|\n|$)", output_str)[:-1]
        start_int, end_int = spans_dict[index_int]
        edits_list.append((start_int, end_int, json.dumps(source_value,
                                                         ensure_ascii=False)))
    for start_int, end_int, replacement_str in reversed(edits_list):
        text_str = text_str[:start_int] + replacement_str + text_str[end_int:]
    return text_str


def validate_notebook_none(original_str: str, output_str: str,
                            path_str: str) -> None:
    """Verify source-only notebook changes and per-cell Python structure.

    Args:
        original_str (str): Original container JSON.
        output_str (str): Proposed container JSON.
        path_str (str): File label for diagnostics.
    Returns:
        None: Raises when metadata, outputs or code semantics differ.
    Warnings:
        Structural equality is not a runtime behavior proof.
    """
    from refactrail.formatting import validate_formatted_none
    original_dict = read_notebook_dict(original_str)
    output_dict = read_notebook_dict(output_str)
    comparable_dict = deepcopy(output_dict)
    if len(original_dict["cells"]) != len(output_dict["cells"]):
        raise ValueError("Formatting changed notebook cell count")
    for index_int, original_cell in enumerate(original_dict["cells"]):
        if original_cell["cell_type"] != "code":
            continue
        output_cell = output_dict["cells"][index_int]
        validate_formatted_none(read_cell_source_str(original_cell, index_int),
            read_cell_source_str(output_cell, index_int),
            f"{path_str}#cell={index_int + 1}")
        comparable_dict["cells"][index_int]["source"] = original_cell["source"]
    if original_dict != comparable_dict:
        raise ValueError("Formatting changed notebook metadata or outputs")


def raise_json_constant_none(constant_str: str) -> None:
    """Refuse nonstandard JSON numeric constants in notebook metadata.

    Args:
        constant_str (str): NaN or an infinity spelling.
    Returns:
        None: Always raises ValueError.
    Warnings:
        Notebook JSON must not silently accept nonstandard constants.
    """
    raise ValueError(f"Unsupported notebook JSON constant: {constant_str}")
