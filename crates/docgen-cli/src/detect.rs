use std::path::Path;
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

/// Returns `true` if the Python source has functions/classes missing docstrings.
pub fn python_has_missing_docstrings(source: &str, force: bool) -> bool {
    let mut parser = Parser::new();
    let lang: Language = tree_sitter_python::LANGUAGE.into();
    parser.set_language(&lang).expect("valid Python grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return false,
    };

    let query_src = r#"
        [(function_definition) (class_definition)] @def
    "#;
    let query = Query::new(&lang, query_src).expect("valid query");
    let mut cursor = QueryCursor::new();
    let src_bytes = source.as_bytes();

    let mut matches = cursor.matches(&query, tree.root_node(), src_bytes);
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let node = capture.node;
            let has_docstring = node
                .child_by_field_name("body")
                .and_then(|b| b.named_child(0))
                .map(|stmt| {
                    if stmt.kind() == "expression_statement" {
                        stmt.named_child(0)
                            .map(|expr| expr.kind() == "string")
                            .unwrap_or(false)
                    } else {
                        false
                    }
                })
                .unwrap_or(false);

            if !has_docstring || force {
                return true;
            }
        }
    }
    false
}

/// Dispatch to Python or TS detector based on file extension.
/// (TS detection added in Task 6 — stub returns false for now)
pub fn needs_docstrings(file: &Path, source: &str, force: bool) -> bool {
    match file.extension().and_then(|e| e.to_str()) {
        Some("py") => python_has_missing_docstrings(source, force),
        Some("ts") | Some("tsx") => false, // implemented in Task 6
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn py(src: &str) -> bool {
        python_has_missing_docstrings(src, false)
    }

    #[test]
    fn py_fn_without_docstring_detected() {
        assert!(py("def foo():\n    pass\n"));
    }

    #[test]
    fn py_fn_with_docstring_not_flagged() {
        assert!(!py("def foo():\n    \"\"\"Docstring.\"\"\"\n    pass\n"));
    }

    #[test]
    fn py_class_without_docstring_detected() {
        assert!(py("class Foo:\n    pass\n"));
    }

    #[test]
    fn py_force_flags_documented_fn() {
        assert!(python_has_missing_docstrings(
            "def foo():\n    \"\"\"Exists.\"\"\"\n    pass\n",
            true
        ));
    }
}
