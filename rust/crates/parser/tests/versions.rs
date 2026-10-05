//! Version-specific syntax against CPython 3.12, 3.13 and 3.14's own
//! results, recorded by scripts/gen_version_cases.py: the tree dump, the
//! compile() outcome and the symbol table must match for each version.

use refactrail_parser::Version;

#[path = "versions/cases_312.rs"]
mod cases_312;
#[path = "versions/cases_313.rs"]
mod cases_313;
#[path = "versions/cases_314.rs"]
mod cases_314;

fn compile_outcome(source: &str, version: Version) -> String {
    match refactrail_parser::compile_check_version(source, version) {
        None => "OK".into(),
        Some((line, offset, message)) => format!("ERROR {line} {offset} {message}"),
    }
}

fn check(version: Version, cases: &[(&str, &str, &str, &str)]) {
    for &(source, tree, outcome, symbols) in cases {
        let actual_tree = refactrail_parser::dump_source_version(source, true, version);
        if tree.starts_with("ERROR") {
            assert!(actual_tree.starts_with("ERROR"), "{version:?} tree should fail: {source:?}");
        } else {
            assert_eq!(actual_tree, tree, "{version:?} tree of {source:?}");
        }
        assert_eq!(compile_outcome(source, version), outcome, "{version:?} compile of {source:?}");
        if symbols != "SKIP" {
            assert_eq!(refactrail_parser::dump_symtable_source_version(source, version), symbols, "{version:?} symtable of {source:?}");
        }
    }
}

#[test]
fn python_3_12_cases_match_cpython() {
    check(Version::Py312, cases_312::CASES);
}

#[test]
fn python_3_13_cases_match_cpython() {
    check(Version::Py313, cases_313::CASES);
}

#[test]
fn python_3_14_cases_match_cpython() {
    check(Version::Py314, cases_314::CASES);
}
