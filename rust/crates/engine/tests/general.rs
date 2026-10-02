//! Regression tests for lint, scope evidence and formatting in the Rust
//! engine. Corpus parity with the Python engine is checked separately by
//! scripts/compare_lint.py, scripts/scope_parity.py and
//! scripts/format_parity.py.

use refactrail_engine::format::format_source;
use refactrail_engine::lint::{lint_file, lint_settings};
use refactrail_engine::notebook::format_notebook;

fn codes(source: &str) -> Vec<(usize, usize, String)> {
    let settings = lint_settings(&["RC".to_string()], &[]);
    lint_file("example.py", source.as_bytes(), &settings).into_iter().map(|row| (row.1, row.2, row.3)).collect()
}

#[test]
fn duplicate_keys_follow_python_equality() {
    let found = codes("d = {1: 'a', True: 'b', 1.0: 'c', 1+0j: 'd'}\n");
    assert!(found.iter().all(|(_, _, code)| code == "RC102"));
    assert_eq!(found.len(), 2);
    let found = codes("d = {0: 1, -0.0: 2, 10**20: 3, 100000000000000000000: 4, 1e20: 5}\n");
    assert_eq!(found.iter().filter(|(_, _, code)| code == "RC102").count(), 2);
    assert!(codes("d = {'a': 1, b'a': 2, (1, 2): 3, (1, 2.5): 4}\n").is_empty());
}

#[test]
fn unresolved_names_and_unused_imports() {
    let found = codes("import os\nimport sys\nprint(sys.path, missing)\n");
    assert_eq!(found, vec![(1, 8, "RC202".to_string()), (3, 17, "RC201".to_string())]);
}

#[test]
fn closures_class_cells_and_comprehensions_resolve() {
    let source = "class Shape:\n    def area(self):\n        return __class__, super()\n\
                  def outer():\n    value = 1\n    def inner():\n        return value\n    return inner\n\
                  squares = [number * number for number in range(3)]\n";
    assert!(codes(source).is_empty());
}

#[test]
fn deeply_nested_valid_code_is_analyzed() {
    // Deep trees need the analysis stack, as the CLI and core provide.
    let source = format!("total = {}missing\n", "1 + ".repeat(1200));
    let found = refactrail_engine::with_analysis_stack(|| codes(&source));
    assert_eq!(found, vec![(1, 4809, "RC201".to_string())]);
}

#[test]
fn spacing_and_wrapping() {
    assert_eq!(format_source("x=[1 ,2]\n", None, false).unwrap(), "x = [1, 2]\n");
    let long = format!("result = compute({})\n", (0..12).map(|index| format!("argument_{index}")).collect::<Vec<_>>().join(", "));
    let wrapped = format_source(&long, Some(60), false).unwrap();
    assert!(wrapped.lines().all(|line| line.chars().count() <= 60));
    assert_eq!(wrapped.replace([' ', '\n'], ""), long.replace([' ', '\n'], ""));
}

#[test]
fn type_ignore_comments_keep_line_structure() {
    let long = format!("result = compute({})  # type: ignore\n", (0..12).map(|index| format!("argument_{index}")).collect::<Vec<_>>().join(", "));
    assert_eq!(format_source(&long, Some(60), false).unwrap(), long);
}

#[test]
fn notebook_sources_are_replaced_in_place() {
    let notebook = "{\"cells\": [{\"cell_type\": \"code\", \"metadata\": {}, \"source\": [\"x=1\\n\", \"y=[1 ,2]\"]}],\n \"metadata\": {}, \"nbformat\": 4, \"nbformat_minor\": 5}";
    let formatted = format_notebook(notebook, "book.ipynb", None, false).unwrap();
    assert_eq!(formatted, notebook.replace("[\"x=1\\n\", \"y=[1 ,2]\"]", "[\"x = 1\\n\", \"y = [1, 2]\\n\"]"));
    let duplicate = "{\"cells\": [], \"cells\": [], \"nbformat\": 4}";
    assert_eq!(format_notebook(duplicate, "book.ipynb", None, false).unwrap_err(), "Notebook contains duplicate JSON keys");
}
