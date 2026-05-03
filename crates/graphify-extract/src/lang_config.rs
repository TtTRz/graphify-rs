//! Per-language configuration for tree-sitter–based extraction.
//!
//! Each [`LanguageConfig`] describes the AST node types that correspond to
//! classes, functions, imports, and calls for a given language. The actual
//! tree-sitter grammar crates are not yet wired in; this module is
//! **configuration only**.

use std::collections::HashSet;

/// Metadata describing how to extract entities from a language's AST.
#[derive(Debug, Clone)]
pub struct LanguageConfig {
    /// Human-readable language name.
    pub name: &'static str,
    /// Crate that provides the tree-sitter grammar (e.g. `"tree_sitter_python"`).
    pub ts_crate: &'static str,
    /// AST node types that represent class-like definitions.
    pub class_types: HashSet<&'static str>,
    /// AST node types that represent function/method definitions.
    pub function_types: HashSet<&'static str>,
    /// AST node types that represent import statements.
    pub import_types: HashSet<&'static str>,
    /// AST node types that represent function calls.
    pub call_types: HashSet<&'static str>,
    /// Field name for the entity name within the AST node.
    pub name_field: &'static str,
    /// Field name for the body of a class/function.
    pub body_field: &'static str,
    /// Field name for the function being called in a call expression.
    pub call_function_field: &'static str,
    /// AST node types that delimit function boundaries (for scope analysis).
    pub function_boundary_types: HashSet<&'static str>,
    /// Whether the language uses parenthesised labels (e.g. Go `func (r *Recv) Name()`).
    pub function_label_parens: bool,
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn hs(items: &[&'static str]) -> HashSet<&'static str> {
    items.iter().copied().collect()
}

// ---------------------------------------------------------------------------
// Language configs (matching the Python extract.py)
// ---------------------------------------------------------------------------

pub fn python_config() -> LanguageConfig {
    LanguageConfig {
        name: "python",
        ts_crate: "tree_sitter_python",
        class_types: hs(&["class_definition"]),
        function_types: hs(&["function_definition"]),
        import_types: hs(&["import_statement", "import_from_statement"]),
        call_types: hs(&["call"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&["function_definition", "class_definition"]),
        function_label_parens: false,
    }
}

pub fn javascript_config() -> LanguageConfig {
    LanguageConfig {
        name: "javascript",
        ts_crate: "tree_sitter_javascript",
        class_types: hs(&["class_declaration", "class"]),
        function_types: hs(&[
            "function_declaration",
            "method_definition",
            "arrow_function",
            "function",
        ]),
        import_types: hs(&["import_statement"]),
        call_types: hs(&["call_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&[
            "function_declaration",
            "method_definition",
            "arrow_function",
            "class_declaration",
        ]),
        function_label_parens: false,
    }
}

pub fn typescript_config() -> LanguageConfig {
    let mut cfg = javascript_config();
    cfg.name = "typescript";
    cfg.ts_crate = "tree_sitter_typescript";
    cfg.class_types.insert("abstract_class_declaration");
    cfg.function_types.insert("function_signature");
    cfg.import_types.insert("import_statement");
    cfg
}

pub fn go_config() -> LanguageConfig {
    LanguageConfig {
        name: "go",
        ts_crate: "tree_sitter_go",
        class_types: hs(&["type_declaration", "type_spec"]),
        function_types: hs(&["function_declaration", "method_declaration"]),
        import_types: hs(&["import_declaration", "import_spec"]),
        call_types: hs(&["call_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&["function_declaration", "method_declaration"]),
        function_label_parens: true,
    }
}

pub fn rust_config() -> LanguageConfig {
    LanguageConfig {
        name: "rust",
        ts_crate: "tree_sitter_rust",
        class_types: hs(&["struct_item", "enum_item", "trait_item", "impl_item"]),
        function_types: hs(&["function_item"]),
        import_types: hs(&["use_declaration"]),
        call_types: hs(&["call_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&["function_item", "impl_item"]),
        function_label_parens: false,
    }
}

pub fn java_config() -> LanguageConfig {
    LanguageConfig {
        name: "java",
        ts_crate: "tree_sitter_java",
        class_types: hs(&[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "annotation_type_declaration",
        ]),
        function_types: hs(&["method_declaration", "constructor_declaration"]),
        import_types: hs(&["import_declaration"]),
        call_types: hs(&["method_invocation"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "name",
        function_boundary_types: hs(&[
            "method_declaration",
            "constructor_declaration",
            "class_declaration",
        ]),
        function_label_parens: false,
    }
}

pub fn c_config() -> LanguageConfig {
    LanguageConfig {
        name: "c",
        ts_crate: "tree_sitter_c",
        class_types: hs(&["struct_specifier", "enum_specifier", "union_specifier"]),
        function_types: hs(&["function_definition"]),
        import_types: hs(&["preproc_include"]),
        call_types: hs(&["call_expression"]),
        name_field: "declarator",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&["function_definition"]),
        function_label_parens: false,
    }
}

pub fn cpp_config() -> LanguageConfig {
    let mut cfg = c_config();
    cfg.name = "cpp";
    cfg.ts_crate = "tree_sitter_cpp";
    cfg.class_types.insert("class_specifier");
    cfg.class_types.insert("namespace_definition");
    cfg.function_types.insert("function_definition");
    cfg
}

pub fn ruby_config() -> LanguageConfig {
    LanguageConfig {
        name: "ruby",
        ts_crate: "tree_sitter_ruby",
        class_types: hs(&["class", "module"]),
        function_types: hs(&["method", "singleton_method"]),
        import_types: hs(&["call"]), // require/include are calls in Ruby
        call_types: hs(&["call", "command"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "method",
        function_boundary_types: hs(&["method", "singleton_method", "class"]),
        function_label_parens: false,
    }
}

pub fn csharp_config() -> LanguageConfig {
    LanguageConfig {
        name: "csharp",
        ts_crate: "tree_sitter_c_sharp",
        class_types: hs(&[
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
            "enum_declaration",
        ]),
        function_types: hs(&["method_declaration", "constructor_declaration"]),
        import_types: hs(&["using_directive"]),
        call_types: hs(&["invocation_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&[
            "method_declaration",
            "constructor_declaration",
            "class_declaration",
        ]),
        function_label_parens: false,
    }
}

pub fn kotlin_config() -> LanguageConfig {
    LanguageConfig {
        name: "kotlin",
        ts_crate: "tree_sitter_kotlin",
        class_types: hs(&[
            "class_declaration",
            "object_declaration",
            "interface_declaration",
        ]),
        function_types: hs(&["function_declaration"]),
        import_types: hs(&["import_header"]),
        call_types: hs(&["call_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&["function_declaration", "class_declaration"]),
        function_label_parens: false,
    }
}

pub fn scala_config() -> LanguageConfig {
    LanguageConfig {
        name: "scala",
        ts_crate: "tree_sitter_scala",
        class_types: hs(&["class_definition", "object_definition", "trait_definition"]),
        function_types: hs(&["function_definition", "val_definition"]),
        import_types: hs(&["import_declaration"]),
        call_types: hs(&["call_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&["function_definition", "class_definition"]),
        function_label_parens: false,
    }
}

pub fn php_config() -> LanguageConfig {
    LanguageConfig {
        name: "php",
        ts_crate: "tree_sitter_php",
        class_types: hs(&[
            "class_declaration",
            "interface_declaration",
            "trait_declaration",
        ]),
        function_types: hs(&["function_definition", "method_declaration"]),
        import_types: hs(&["namespace_use_declaration"]),
        call_types: hs(&["function_call_expression", "method_call_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&[
            "function_definition",
            "method_declaration",
            "class_declaration",
        ]),
        function_label_parens: false,
    }
}

pub fn swift_config() -> LanguageConfig {
    LanguageConfig {
        name: "swift",
        ts_crate: "tree_sitter_swift",
        class_types: hs(&[
            "class_declaration",
            "struct_declaration",
            "protocol_declaration",
            "enum_declaration",
        ]),
        function_types: hs(&["function_declaration", "init_declaration"]),
        import_types: hs(&["import_declaration"]),
        call_types: hs(&["call_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&["function_declaration", "class_declaration"]),
        function_label_parens: false,
    }
}

pub fn lua_config() -> LanguageConfig {
    LanguageConfig {
        name: "lua",
        ts_crate: "tree_sitter_lua",
        class_types: hs(&[]),
        function_types: hs(&[
            "function_declaration",
            "local_function_declaration",
            "function_definition",
        ]),
        import_types: hs(&[]), // require() is a call
        call_types: hs(&["function_call"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "name",
        function_boundary_types: hs(&["function_declaration", "local_function_declaration"]),
        function_label_parens: false,
    }
}

pub fn zig_config() -> LanguageConfig {
    LanguageConfig {
        name: "zig",
        ts_crate: "tree_sitter_zig",
        class_types: hs(&["container_declaration"]),
        function_types: hs(&["fn_proto"]),
        import_types: hs(&[]),
        call_types: hs(&["call_expr"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&["fn_proto"]),
        function_label_parens: false,
    }
}

pub fn powershell_config() -> LanguageConfig {
    LanguageConfig {
        name: "powershell",
        ts_crate: "tree_sitter_powershell",
        class_types: hs(&["class_statement"]),
        function_types: hs(&["function_statement"]),
        import_types: hs(&["using_statement"]),
        call_types: hs(&["command_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "name",
        function_boundary_types: hs(&["function_statement"]),
        function_label_parens: false,
    }
}

pub fn elixir_config() -> LanguageConfig {
    LanguageConfig {
        name: "elixir",
        ts_crate: "tree_sitter_elixir",
        class_types: hs(&["call"]),    // defmodule is a call in Elixir
        function_types: hs(&["call"]), // def/defp are calls too
        import_types: hs(&["call"]),   // import/use/require
        call_types: hs(&["call"]),
        name_field: "target",
        body_field: "body",
        call_function_field: "target",
        function_boundary_types: hs(&["call"]),
        function_label_parens: false,
    }
}

pub fn objc_config() -> LanguageConfig {
    LanguageConfig {
        name: "objc",
        ts_crate: "tree_sitter_objc",
        class_types: hs(&[
            "class_interface",
            "class_implementation",
            "protocol_declaration",
        ]),
        function_types: hs(&["method_definition", "function_definition"]),
        import_types: hs(&["preproc_import", "preproc_include"]),
        call_types: hs(&["message_expression", "call_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "selector",
        function_boundary_types: hs(&["method_definition", "function_definition"]),
        function_label_parens: false,
    }
}

pub fn julia_config() -> LanguageConfig {
    LanguageConfig {
        name: "julia",
        ts_crate: "tree_sitter_julia",
        class_types: hs(&["struct_definition", "abstract_definition"]),
        function_types: hs(&["function_definition", "short_function_definition"]),
        import_types: hs(&["import_statement", "using_statement"]),
        call_types: hs(&["call_expression"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&["function_definition"]),
        function_label_parens: false,
    }
}

pub fn dart_config() -> LanguageConfig {
    LanguageConfig {
        name: "dart",
        ts_crate: "tree_sitter_dart",
        class_types: hs(&[
            "class_definition",
            "enum_declaration",
            "mixin_declaration",
            "extension_declaration",
        ]),
        function_types: hs(&[
            "function_signature",
            "method_signature",
            "function_body",
            "method_definition",
        ]),
        import_types: hs(&["import_or_export"]),
        call_types: hs(&["method_invocation", "function_expression_invocation"]),
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        function_boundary_types: hs(&[
            "method_definition",
            "function_signature",
            "class_definition",
        ]),
        function_label_parens: false,
    }
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// Return the [`LanguageConfig`] for the given language name.
pub fn config_for_language(lang: &str) -> Option<LanguageConfig> {
    match lang {
        "python" => Some(python_config()),
        "javascript" => Some(javascript_config()),
        "typescript" => Some(typescript_config()),
        "go" => Some(go_config()),
        "rust" => Some(rust_config()),
        "java" => Some(java_config()),
        "c" => Some(c_config()),
        "cpp" => Some(cpp_config()),
        "ruby" => Some(ruby_config()),
        "csharp" => Some(csharp_config()),
        "kotlin" => Some(kotlin_config()),
        "scala" => Some(scala_config()),
        "php" => Some(php_config()),
        "swift" => Some(swift_config()),
        "lua" => Some(lua_config()),
        "zig" => Some(zig_config()),
        "powershell" => Some(powershell_config()),
        "elixir" => Some(elixir_config()),
        "objc" => Some(objc_config()),
        "julia" => Some(julia_config()),
        "dart" => Some(dart_config()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_regex_languages_have_configs() {
        let languages = [
            "python",
            "javascript",
            "typescript",
            "go",
            "rust",
            "java",
            "c",
            "cpp",
            "ruby",
            "csharp",
            "kotlin",
            "scala",
            "php",
            "swift",
            "lua",
            "zig",
            "powershell",
            "elixir",
            "objc",
            "julia",
            "dart",
        ];
        for lang in languages {
            assert!(
                config_for_language(lang).is_some(),
                "missing config for {lang}"
            );
        }
        assert_eq!(languages.len(), 21);
    }

    #[test]
    fn python_config_has_expected_types() {
        let cfg = python_config();
        assert!(cfg.class_types.contains("class_definition"));
        assert!(cfg.function_types.contains("function_definition"));
        assert!(cfg.import_types.contains("import_statement"));
        assert!(cfg.import_types.contains("import_from_statement"));
    }

    #[test]
    fn typescript_extends_javascript() {
        let ts = typescript_config();
        assert!(ts.class_types.contains("abstract_class_declaration"));
        // Inherited from JS
        assert!(ts.class_types.contains("class_declaration"));
    }

    #[test]
    fn unknown_language_returns_none() {
        assert!(config_for_language("brainfuck").is_none());
    }
}
