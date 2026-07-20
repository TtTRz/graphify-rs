//! Tree-sitter language configurations.
//!
//! Each supported language has a `TsConfig` describing which tree-sitter node
//! kinds correspond to classes, functions, imports, and calls.

use std::sync::LazyLock;

use tree_sitter::Language;

/// Describes which tree-sitter node kinds correspond to classes, functions,
/// imports and calls for a given language.
///
/// Node-kind sets are small (at most a handful of entries), so a linear scan
/// over a static slice is cheaper than hashing into a `HashSet` — and, more
/// importantly, each language's config is built exactly once (via `LazyLock`)
/// rather than re-allocated for every file that gets extracted.
pub struct TsConfig {
    pub class_types: &'static [&'static str],
    pub function_types: &'static [&'static str],
    pub import_types: &'static [&'static str],
    pub call_types: &'static [&'static str],
    pub name_field: &'static str,
    pub class_name_field: Option<&'static str>,
    pub body_field: &'static str,
    pub call_function_field: &'static str,
}

/// Resolve a language identifier to its tree-sitter `Language` and `TsConfig`.
/// Returns `None` for unsupported languages.
///
/// Each `TsConfig` is built once per language and cached for the lifetime of
/// the process, so calling this repeatedly (e.g. once per extracted file)
/// does not re-allocate the underlying configuration.
pub fn resolve_language(lang: &str) -> Option<(Language, &'static TsConfig)> {
    match lang {
        "python" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(python_config);
            Some((tree_sitter_python::LANGUAGE.into(), &CONFIG))
        }
        "javascript" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(js_config);
            Some((tree_sitter_javascript::LANGUAGE.into(), &CONFIG))
        }
        "typescript" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(js_config);
            Some((tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), &CONFIG))
        }
        "tsx" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(js_config);
            Some((tree_sitter_typescript::LANGUAGE_TSX.into(), &CONFIG))
        }
        "rust" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(rust_config);
            Some((tree_sitter_rust::LANGUAGE.into(), &CONFIG))
        }
        "go" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(go_config);
            Some((tree_sitter_go::LANGUAGE.into(), &CONFIG))
        }
        "java" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(java_config);
            Some((tree_sitter_java::LANGUAGE.into(), &CONFIG))
        }
        "c" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(c_config);
            Some((tree_sitter_c::LANGUAGE.into(), &CONFIG))
        }
        "cpp" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(cpp_config);
            Some((tree_sitter_cpp::LANGUAGE.into(), &CONFIG))
        }
        "ruby" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(ruby_config);
            Some((tree_sitter_ruby::LANGUAGE.into(), &CONFIG))
        }
        "csharp" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(csharp_config);
            Some((tree_sitter_c_sharp::LANGUAGE.into(), &CONFIG))
        }
        "dart" => {
            static CONFIG: LazyLock<TsConfig> = LazyLock::new(dart_config);
            Some((tree_sitter_dart::LANGUAGE.into(), &CONFIG))
        }
        _ => None,
    }
}

fn python_config() -> TsConfig {
    TsConfig {
        class_types: &["class_definition"],
        function_types: &["function_definition"],
        import_types: &["import_statement", "import_from_statement"],
        call_types: &["call"],
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn js_config() -> TsConfig {
    TsConfig {
        class_types: &["class_declaration", "class"],
        function_types: &[
            "function_declaration",
            "method_definition",
            "arrow_function",
            "generator_function_declaration",
            "generator_function",
            "async_function_declaration",
        ],
        import_types: &["import_statement"],
        call_types: &["call_expression"],
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn rust_config() -> TsConfig {
    TsConfig {
        class_types: &["struct_item", "enum_item", "trait_item", "impl_item"],
        function_types: &["function_item"],
        import_types: &["use_declaration"],
        call_types: &["call_expression"],
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn go_config() -> TsConfig {
    TsConfig {
        class_types: &["type_declaration"],
        function_types: &["function_declaration", "method_declaration"],
        import_types: &["import_declaration"],
        call_types: &["call_expression"],
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn java_config() -> TsConfig {
    TsConfig {
        class_types: &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
        ],
        function_types: &["method_declaration", "constructor_declaration"],
        import_types: &["import_declaration"],
        call_types: &["method_invocation"],
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "name",
    }
}

fn c_config() -> TsConfig {
    TsConfig {
        class_types: &["struct_specifier", "enum_specifier", "type_definition"],
        function_types: &["function_definition"],
        import_types: &["preproc_include"],
        call_types: &["call_expression"],
        name_field: "declarator",
        class_name_field: Some("name"),
        body_field: "body",
        call_function_field: "function",
    }
}

fn cpp_config() -> TsConfig {
    TsConfig {
        class_types: &[
            "class_specifier",
            "struct_specifier",
            "enum_specifier",
            "namespace_definition",
        ],
        function_types: &["function_definition"],
        import_types: &["preproc_include"],
        call_types: &["call_expression"],
        name_field: "declarator",
        class_name_field: Some("name"),
        body_field: "body",
        call_function_field: "function",
    }
}

fn ruby_config() -> TsConfig {
    TsConfig {
        class_types: &["class", "module"],
        function_types: &["method", "singleton_method"],
        import_types: &["call"],
        call_types: &["call"],
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "method",
    }
}

fn csharp_config() -> TsConfig {
    TsConfig {
        class_types: &[
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
            "enum_declaration",
        ],
        function_types: &["method_declaration", "constructor_declaration"],
        import_types: &["using_directive"],
        call_types: &["invocation_expression"],
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn dart_config() -> TsConfig {
    TsConfig {
        class_types: &[
            "class_definition",
            "enum_declaration",
            "mixin_declaration",
            "extension_declaration",
        ],
        function_types: &[
            "function_signature",
            "method_signature",
            "function_body",
            "function_declaration",
            "method_definition",
        ],
        import_types: &["import_or_export", "part_directive", "part_of_directive"],
        call_types: &["method_invocation", "function_expression_invocation"],
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}
