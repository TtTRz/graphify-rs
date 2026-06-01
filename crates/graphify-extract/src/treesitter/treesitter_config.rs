//! Tree-sitter language configurations.
//!
//! Each supported language has a `TsConfig` describing which tree-sitter node
//! kinds correspond to classes, functions, imports, and calls.

use std::collections::HashSet;

use tree_sitter::Language;

/// Describes which tree-sitter node kinds correspond to classes, functions,
/// imports and calls for a given language.
pub struct TsConfig {
    pub class_types: HashSet<&'static str>,
    pub function_types: HashSet<&'static str>,
    pub import_types: HashSet<&'static str>,
    pub call_types: HashSet<&'static str>,
    pub name_field: &'static str,
    pub class_name_field: Option<&'static str>,
    pub body_field: &'static str,
    pub call_function_field: &'static str,
}

/// Resolve a language identifier to its tree-sitter `Language` and `TsConfig`.
/// Returns `None` for unsupported languages.
pub fn resolve_language(lang: &str) -> Option<(Language, TsConfig)> {
    match lang {
        "python" => Some((tree_sitter_python::LANGUAGE.into(), python_config())),
        "javascript" => Some((tree_sitter_javascript::LANGUAGE.into(), js_config())),
        "typescript" => Some((
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            js_config(),
        )),
        "tsx" => Some((tree_sitter_typescript::LANGUAGE_TSX.into(), js_config())),
        "rust" => Some((tree_sitter_rust::LANGUAGE.into(), rust_config())),
        "go" => Some((tree_sitter_go::LANGUAGE.into(), go_config())),
        "java" => Some((tree_sitter_java::LANGUAGE.into(), java_config())),
        "c" => Some((tree_sitter_c::LANGUAGE.into(), c_config())),
        "cpp" => Some((tree_sitter_cpp::LANGUAGE.into(), cpp_config())),
        "ruby" => Some((tree_sitter_ruby::LANGUAGE.into(), ruby_config())),
        "csharp" => Some((tree_sitter_c_sharp::LANGUAGE.into(), csharp_config())),
        "dart" => Some((tree_sitter_dart::LANGUAGE.into(), dart_config())),
        _ => None,
    }
}

fn python_config() -> TsConfig {
    TsConfig {
        class_types: ["class_definition"].into_iter().collect(),
        function_types: ["function_definition"].into_iter().collect(),
        import_types: ["import_statement", "import_from_statement"]
            .into_iter()
            .collect(),
        call_types: ["call"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn js_config() -> TsConfig {
    TsConfig {
        class_types: ["class_declaration", "class"].into_iter().collect(),
        function_types: [
            "function_declaration",
            "method_definition",
            "arrow_function",
            "generator_function_declaration",
            "generator_function",
            "async_function_declaration",
        ]
        .into_iter()
        .collect(),
        import_types: ["import_statement"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn rust_config() -> TsConfig {
    TsConfig {
        class_types: ["struct_item", "enum_item", "trait_item", "impl_item"]
            .into_iter()
            .collect(),
        function_types: ["function_item"].into_iter().collect(),
        import_types: ["use_declaration"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn go_config() -> TsConfig {
    TsConfig {
        class_types: ["type_declaration"].into_iter().collect(),
        function_types: ["function_declaration", "method_declaration"]
            .into_iter()
            .collect(),
        import_types: ["import_declaration"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn java_config() -> TsConfig {
    TsConfig {
        class_types: [
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
        ]
        .into_iter()
        .collect(),
        function_types: ["method_declaration", "constructor_declaration"]
            .into_iter()
            .collect(),
        import_types: ["import_declaration"].into_iter().collect(),
        call_types: ["method_invocation"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "name",
    }
}

fn c_config() -> TsConfig {
    TsConfig {
        class_types: ["struct_specifier", "enum_specifier", "type_definition"]
            .into_iter()
            .collect(),
        function_types: ["function_definition"].into_iter().collect(),
        import_types: ["preproc_include"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "declarator",
        class_name_field: Some("name"),
        body_field: "body",
        call_function_field: "function",
    }
}

fn cpp_config() -> TsConfig {
    TsConfig {
        class_types: [
            "class_specifier",
            "struct_specifier",
            "enum_specifier",
            "namespace_definition",
        ]
        .into_iter()
        .collect(),
        function_types: ["function_definition"].into_iter().collect(),
        import_types: ["preproc_include"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "declarator",
        class_name_field: Some("name"),
        body_field: "body",
        call_function_field: "function",
    }
}

fn ruby_config() -> TsConfig {
    TsConfig {
        class_types: ["class", "module"].into_iter().collect(),
        function_types: ["method", "singleton_method"].into_iter().collect(),
        import_types: ["call"].into_iter().collect(),
        call_types: ["call"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "method",
    }
}

fn csharp_config() -> TsConfig {
    TsConfig {
        class_types: [
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
            "enum_declaration",
        ]
        .into_iter()
        .collect(),
        function_types: ["method_declaration", "constructor_declaration"]
            .into_iter()
            .collect(),
        import_types: ["using_directive"].into_iter().collect(),
        call_types: ["invocation_expression"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn dart_config() -> TsConfig {
    TsConfig {
        class_types: [
            "class_definition",
            "enum_declaration",
            "mixin_declaration",
            "extension_declaration",
        ]
        .into_iter()
        .collect(),
        function_types: [
            "function_signature",
            "method_signature",
            "function_body",
            "function_declaration",
            "method_definition",
        ]
        .into_iter()
        .collect(),
        import_types: ["import_or_export", "part_directive", "part_of_directive"]
            .into_iter()
            .collect(),
        call_types: ["method_invocation", "function_expression_invocation"]
            .into_iter()
            .collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}
