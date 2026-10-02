"""Intentional correctness findings; inspect with lint rather than run."""


def collect_entries(entries=[]):
    """Illustrate a shared mutable default requiring review."""
    return entries


LABELS = {1: "first", True: "overwritten"}
