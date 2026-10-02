"""Locate JSON values while preserving all unrelated container bytes."""

import json
import re

JSON_DECODER = json.JSONDecoder()
JSON_SPACE_PATTERN = re.compile(r"\s*")


def skip_space_int(text_str: str, offset_int: int) -> int:
    """Find the next JSON token boundary.

    Args:
        text_str (str): Valid JSON text.
        offset_int (int): Current character offset.
    Returns:
        int: Offset after JSON whitespace.
    Warnings:
        The complete document must be validated before span collection.
    """
    return JSON_SPACE_PATTERN.match(text_str, offset_int).end()


def collect_json_spans_dict(text_str: str) -> dict:
    """Index notebook source-value spans without reserializing metadata.

    Args:
        text_str (str): Validated JSON document.
    Returns:
        dict: Cell index to source-value character span.
    Warnings:
        Duplicate keys must be rejected by the document loader first.
    """
    spans_dict = {}
    collect_value_int(text_str, skip_space_int(text_str, 0), (), spans_dict)
    return spans_dict


def collect_value_int(text_str: str, offset_int: int, path_tuple: tuple,
                       spans_dict: dict) -> int:
    """Walk one JSON value and record code-cell source locations.

    Args:
        text_str (str): Validated JSON text.
        offset_int (int): Start of a value.
        path_tuple (tuple): Current object keys and array indexes.
        spans_dict (dict): Collected source-value spans.
    Returns:
        int: Exclusive end of the value.
    Warnings:
        Recursion follows document structure without evaluating source.
    """
    start_int = offset_int
    if text_str[offset_int] not in "[{":
        _, offset_int = JSON_DECODER.raw_decode(text_str, offset_int)
    else:
        offset_int = collect_container_int(text_str, offset_int,
                                            path_tuple, spans_dict)
    if len(path_tuple) == 3 and path_tuple[0] == "cells" and (
        path_tuple[2] == "source"
    ):
        spans_dict[path_tuple[1]] = (start_int, offset_int)
    return offset_int


def collect_container_int(text_str: str, offset_int: int,
                           path_tuple: tuple, spans_dict: dict) -> int:
    """Walk object members or array entries retaining original positions.

    Args:
        text_str (str): Validated JSON text.
        offset_int (int): Opening brace or bracket.
        path_tuple (tuple): Parent JSON path.
        spans_dict (dict): Source spans accumulated in place.
    Returns:
        int: Exclusive container end.
    Warnings:
        Called only after standard-library JSON validation.
    """
    object_bool = text_str[offset_int] == "{"
    closing_str = "}" if object_bool else "]"
    offset_int = skip_space_int(text_str, offset_int + 1)
    entry_int = 0
    while text_str[offset_int] != closing_str:
        key_value = entry_int
        if object_bool:
            key_value, offset_int = JSON_DECODER.raw_decode(
                text_str, offset_int)
            offset_int = skip_space_int(text_str, offset_int)
            offset_int = skip_space_int(text_str, offset_int + 1)
        offset_int = collect_value_int(text_str, offset_int,
                                        (*path_tuple, key_value), spans_dict)
        offset_int = skip_space_int(text_str, offset_int)
        if text_str[offset_int] == ",":
            offset_int = skip_space_int(text_str, offset_int + 1)
        entry_int += 1
    return offset_int + 1
