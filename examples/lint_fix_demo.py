"""A small file with findings that `refactrail lint --fix` can fix.

Used by docs/media/refactrail-lint-fix.tape; the recording copies it to a
temporary folder first, so this file keeps its findings.
"""

import os, sys
import json

readings = [21.5, 23.0, 19.8];


def describe_reading(value):
    if not value is None:
        return f"reading" + " " + str(value)
    return "missing"


if not "--quiet" in sys.argv:
    print(describe_reading(readings[0]))
