//! Edge cases whose CPython token dumps were recorded with Python 3.12.

use refactrail_lexer::dump;

fn check(source: &str, expected: &[&str]) {
    let actual = dump(source);
    let lines: Vec<&str> = actual.lines().collect();
    assert_eq!(lines, expected, "source: {source:?}");
}

#[test]
fn missing_final_newline_gets_empty_newline_and_endmarker_row() {
    check("y=2", &[
        "NAME 1,0-1,1 \"y\"", "OP 1,1-1,2 \"=\"", "NUMBER 1,2-1,3 \"2\"",
        "NEWLINE 1,3-1,4 \"\"", "ENDMARKER 2,0-2,0 \"\"",
    ]);
    check("", &["ENDMARKER 1,0-1,0 \"\""]);
    check("# hi", &["COMMENT 1,0-1,4 \"# hi\"", "NL 1,4-1,5 \"\"", "ENDMARKER 2,0-2,0 \"\""]);
}

#[test]
fn doubled_braces_and_empty_spec_middle() {
    check("f\"a{{b}}c{x!r:>{w}}\"\n", &[
        "FSTRING_START 1,0-1,2 \"f\\\"\"",
        "FSTRING_MIDDLE 1,2-1,4 \"a{\"",
        "FSTRING_MIDDLE 1,5-1,7 \"b}\"",
        "FSTRING_MIDDLE 1,8-1,9 \"c\"",
        "OP 1,9-1,10 \"{\"", "NAME 1,10-1,11 \"x\"", "OP 1,11-1,12 \"!\"",
        "NAME 1,12-1,13 \"r\"", "OP 1,13-1,14 \":\"",
        "FSTRING_MIDDLE 1,14-1,15 \">\"",
        "OP 1,15-1,16 \"{\"", "NAME 1,16-1,17 \"w\"", "OP 1,17-1,18 \"}\"",
        "FSTRING_MIDDLE 1,18-1,18 \"\"", "OP 1,18-1,19 \"}\"",
        "FSTRING_END 1,19-1,20 \"\\\"\"", "NEWLINE 1,20-1,21 \"\\n\"",
        "ENDMARKER 2,0-2,0 \"\"",
    ]);
}

#[test]
fn named_escape_ends_the_text_token() {
    check("f\"\\N{DEGREE SIGN}, \"\n", &[
        "FSTRING_START 1,0-1,2 \"f\\\"\"",
        "FSTRING_MIDDLE 1,2-1,17 \"\\\\N{DEGREE SIGN}\"",
        "FSTRING_MIDDLE 1,17-1,19 \", \"",
        "FSTRING_END 1,19-1,20 \"\\\"\"", "NEWLINE 1,20-1,21 \"\\n\"",
        "ENDMARKER 2,0-2,0 \"\"",
    ]);
}

#[test]
fn dedents_tabs_and_columns_in_code_points() {
    check("if a:\n\tb\n\tc\n", &[
        "NAME 1,0-1,2 \"if\"", "NAME 1,3-1,4 \"a\"", "OP 1,4-1,5 \":\"",
        "NEWLINE 1,5-1,6 \"\\n\"", "INDENT 2,0-2,1 \"\\t\"", "NAME 2,1-2,2 \"b\"",
        "NEWLINE 2,2-2,3 \"\\n\"", "NAME 3,1-3,2 \"c\"", "NEWLINE 3,2-3,3 \"\\n\"",
        "DEDENT 4,0-4,0 \"\"", "ENDMARKER 4,0-4,0 \"\"",
    ]);
    check("é = 'ü'\n", &[
        "NAME 1,0-1,1 \"é\"", "OP 1,2-1,3 \"=\"", "STRING 1,4-1,7 \"'ü'\"",
        "NEWLINE 1,7-1,8 \"\\n\"", "ENDMARKER 2,0-2,0 \"\"",
    ]);
}

#[test]
fn number_literals_follow_cpython() {
    let number_texts = |source: &str| -> Vec<String> {
        dump(source).lines().filter(|line| line.starts_with("NUMBER") || line.starts_with("NAME") || line.starts_with("ERROR"))
            .map(|line| line.split(' ').last().unwrap_or("").to_string()).collect()
    };
    for (source, expected) in [
        ("0x1g\n", vec!["\"0x1\"", "\"g\""]),
        ("0x_1\n", vec!["\"0x_1\""]),
        ("1e_5\n", vec!["\"1\"", "\"e_5\""]),
        ("1._5\n", vec!["\"1.\"", "\"_5\""]),
        ("0x1for\n", vec!["\"0x1f\"", "\"or\""]),
        ("0123\n", vec!["\"0123\""]),
        ("1.2.3\n", vec!["\"1.2\"", "\".3\""]),
    ] {
        assert_eq!(number_texts(source), expected, "{source:?}");
    }
    for source in ["0x\n", "0x1_\n", "0o8\n", "0b2\n", "1__2\n", "1_\n", "1e+\n", ".5_\n", "1_e5\n"] {
        assert_eq!(dump(source), "ERROR 1\n", "{source:?}");
    }
}

#[test]
fn newline_ends_a_single_quoted_format_spec() {
    check("f\"{a:x\nb}\"\n", &[
        "FSTRING_START 1,0-1,2 \"f\\\"\"", "OP 1,2-1,3 \"{\"", "NAME 1,3-1,4 \"a\"",
        "OP 1,4-1,5 \":\"", "FSTRING_MIDDLE 1,5-1,6 \"x\"", "NL 1,6-1,7 \"\\n\"",
        "NAME 2,0-2,1 \"b\"", "OP 2,1-2,2 \"}\"", "FSTRING_END 2,2-2,3 \"\\\"\"",
        "NEWLINE 2,3-2,4 \"\\n\"", "ENDMARKER 3,0-3,0 \"\"",
    ]);
    assert_eq!(dump("x = f\"{a\"b\"\ndef g(a, b={}):\n    pass\n"), "ERROR 3\n");
}

#[test]
fn field_brackets_count_every_closer_as_cpython_does() {
    // A stray `)` closes the field's `{` count, so `}` is then single.
    assert_eq!(dump("f\"{a)}\"\n"), "ERROR 1\n");
    // ...but a following opener restores it and the field closes.
    assert!(dump("f\"{a)(}\"\n").contains("FSTRING_END 1,7-1,8"));
    // With the bracket level back at 0 the file can end normally.
    assert!(dump("x = (f\"{a)\n)\ny = 2\n").ends_with("ENDMARKER 4,0-4,0 \"\"\n"));
    // After a nested field the spec continues: `}}` is not an escape.
    assert_eq!(dump("f\"{a:x{b}}}\"\n"), "ERROR 1\n");
    let named = dump("f\"{a:\\N{DASH}}\"\n");
    assert!(named.contains("FSTRING_MIDDLE 1,5-1,13 \"\\\\N{DASH}\""), "{named}");
}

#[test]
fn unterminated_fstring_reports_its_start_line() {
    assert_eq!(dump("x = f\"a\\\nb\n"), "ERROR 1\n");
    assert_eq!(dump("x = (f\"{\n1}a\n"), "ERROR 1\n");
}

#[test]
fn first_backslash_continuation_sets_the_indentation() {
    // Column 0 at the backslash: whitespace keeps counting (no DEDENT).
    let kept = dump("if a:\n    b\n\\\n    c\nd\n");
    assert!(kept.contains("NEWLINE 2,5-2,6 \"\\n\"\nNAME 4,4-4,5 \"c\""), "{kept}");
    // A nonzero column at the backslash wins over the next line's.
    let deeper = dump("if a:\n    b\n        \\\n    c\n");
    assert!(deeper.contains("INDENT 4,0-4,4 \"    \""), "{deeper}");
    assert_eq!(dump("if a:\n    b\n  \\\nc\n"), "ERROR 4\n");
}

#[test]
fn errors_are_reported_with_a_line() {
    assert_eq!(dump("x = 'abc\n"), "ERROR 1\n");
    assert_eq!(dump("x = (1,\n"), "ERROR 1\n");
    assert_eq!(dump("a = 1\nx = '''abc\ndef\n"), "ERROR 2\n");
    assert_eq!(dump("if a:\n        b\n\tc\n"), "ERROR 3\n");
    assert!(dump("x = (1]\n").starts_with("NAME"));
    assert!(dump("x = $\n").contains("OP 1,4-1,5 \"$\""));
    assert_eq!(dump("if a:\n    b\n  c\n"), "ERROR 3\n");
}
