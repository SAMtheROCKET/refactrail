# SQL resources and the reference project style

The tools support a Python entry point, reusable modules, centralized
constants, and query text kept in `sql/*.sql` beside the Python module that
loads it. The architecture view shows function inputs and outputs, branches,
loops, module ownership, and file/database reads. Domain names still come
from source or user context; they are not invented from numbers.

## Draft SQL resources

Preview without writing:

```powershell
python -m funcloom sql report.py
python -m refactrail sql report.py
```

Write a new copy with `src/report.py` and `src/sql/*.sql`:

```powershell
python -m funcloom sql report.py --output report_draft
```

For an existing repository, the same command preserves its directory
layout, imports and resources. An existing `src` stays `src`; Python files
in another folder get an adjacent `sql` folder. Existing SQL assets are
copied unchanged. The destination must be new and outside the input repo:

```powershell
python -m refactrail sql existing_repo --output repo_draft
```

To modularize a script or notebook and extract SQL in one operation:

```powershell
python -m funcloom modularize report.py --package-name src --externalize-sql --output report_package
python -m funcloom modularize analysis.ipynb --package-name src --externalize-sql --output notebook_package
python -m funcloom modularize existing_repo --externalize-sql --output modular_repo
```

The single-program package includes `main.py`, `src/pipeline.py`, generated
function/step/configuration modules as needed, and `src/sql/*.sql`. Run
`python report_package/main.py`. The SQL loader resolves resources relative
to its module, independently of the working directory. Other original file
paths retain the normal modularization limitations.

The SQL flags are opt-in because moving literals to files adds runtime file
I/O and a possible failure when a resource is missing or unreadable. Keep
SQL files with their modules. If packaging generated code into a wheel,
include `sql/*.sql` in that package's package-data configuration.

## Supported extraction and limits

- Detection examines assignments whose underscore-separated name includes
  `query` or `sql`, and first/SQL keyword arguments of `execute`,
  `executemany`, `executescript`, `read_sql`, and `read_sql_query`.
- A complete string must start with a supported SQL keyword (SELECT, WITH,
  INSERT, UPDATE, DELETE, CREATE, ALTER, DROP, MERGE, REPLACE, TRUNCATE,
  EXPLAIN, PRAGMA), optionally after SQL comments. This is a heuristic;
  SQL syntax and the called function's runtime identity are not validated.
- The literal base of `.format()`, `.format_map()` and `%` formatting is
  supported. The original formatting operation, expressions and database
  parameter arguments remain in Python. This does **not** make template
  interpolation safe for untrusted values or SQL identifiers.
- Assets retain the exact decoded string, including escaped CRLFs, Unicode
  and absence of a final newline. SQL is neither reformatted nor executed.
- F-strings, computed concatenation, and other dynamic fragments remain
  inline with SQL002 and the original source line. Existing query loaders
  remain unchanged. General query builders cannot be resolved statically.
- Statements with tool-directive comments and inter-literal comments stay
  in place (SQL003). SQL001 refuses invalid
  Python or a failed structural reconstruction check. A compile failure
  prevents writing the complete SQL draft.
- The generated module is compiled. Replacing loader calls with their
  original literal values must reconstruct the original syntax tree after
  removing the generated helper. This does not prove runtime equivalence:
  I/O, reflection, tracebacks and object lifetimes can differ.
- `sql` accepts UTF-8 Python files or folders; for notebooks use
  `modularize --externalize-sql`. Source files are limited to 2 MB and
  copied projects to 200 MB. Linked files, linked folders, environments
  and standard build/VCS folders are skipped in SQL folder mode.
- Folder mode copies ordinary project resources; review the copy before
  distribution. The tool does not turn a private project into a public
  release or decide which data should be published.

`--format json` emits `sql-assets-1`: source hashes, extracted assets with
source locations, diagnostics, and explicit review/behavior flags. In the
`sql` command, locations refer to original project files. In
`modularize --externalize-sql`, SQL locations refer to the generated Python
modules before extraction; original program locations remain in step records.
`plan_sql_source` and `draft_sql_project_report` are experimental public
Python APIs, shared by FuncLoom and RefacTrail. FuncLoom has no RefacTrail
runtime dependency; the SQL command does not alter native lint rules.

## Architecture outputs

```powershell
python -m flowblueprint report_package --level summary -o high_level.drawio
python -m flowblueprint report_package/main.py --level detailed -o detailed.drawio
python -m flowblueprint report_package/main.py -o architecture.drawio.png
```

The last form requires FlowBlueprint's existing optional PNG renderer
(`flowblueprint[png]`). It embeds the full editable, multi-page draw.io XML
in the PNG, matching the reference files' `mxfile` metadata convention.
Plain `.png` remains an image-only export. Source-based SQL loader evidence
is shown as a file read, separately from database operations. It does not
infer table lineage or runtime-generated query paths.

See `examples/sql_inline.py` for a synthetic, standard-library-only example.
The private reference projects and their business data are not distributed
with these tools.
