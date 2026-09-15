use std::path::Path;

use crate::model::Language;

pub fn detect_language(path: &Path) -> Option<Language> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "rs" => Some(Language::Rust),
        "ts" | "mts" | "cts" => Some(Language::TypeScript),
        "tsx" => Some(Language::Tsx),
        "js" | "mjs" | "cjs" | "jsx" => Some(Language::JavaScript),
        "py" | "pyi" => Some(Language::Python),
        _ => None,
    }
}

pub fn configure_parser(
    parser: &mut tree_sitter::Parser,
    language: Language,
) -> Result<(), tree_sitter::LanguageError> {
    let grammar = match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
    };
    parser.set_language(&grammar)
}

pub fn structural_kind(
    language: Language,
    node_kind: &str,
    parent_kind: Option<&str>,
) -> Option<&'static str> {
    match language {
        Language::Rust => match node_kind {
            "function_item" if matches!(parent_kind, Some("impl_item") | Some("trait_item")) => {
                Some("method")
            }
            "function_item" => Some("function"),
            "struct_item" => Some("struct"),
            "enum_item" => Some("enum"),
            "trait_item" => Some("trait"),
            "impl_item" => Some("impl"),
            "mod_item" => Some("module"),
            "type_item" => Some("type"),
            "const_item" | "static_item" => Some("declaration"),
            "use_declaration" => Some("import"),
            _ => None,
        },
        Language::TypeScript | Language::Tsx => match node_kind {
            "function_declaration" | "generator_function_declaration" => Some("function"),
            "method_definition" | "abstract_method_signature" | "method_signature" => {
                Some("method")
            }
            "class_declaration" | "abstract_class_declaration" => Some("class"),
            "interface_declaration" => Some("interface"),
            "type_alias_declaration" => Some("type"),
            "enum_declaration" => Some("enum"),
            "lexical_declaration" | "variable_declaration" => Some("declaration"),
            "import_statement" | "import_alias" => Some("import"),
            _ => None,
        },
        Language::JavaScript => match node_kind {
            "function_declaration" | "generator_function_declaration" => Some("function"),
            "method_definition" => Some("method"),
            "class_declaration" => Some("class"),
            "lexical_declaration" | "variable_declaration" => Some("declaration"),
            "import_statement" => Some("import"),
            _ => None,
        },
        Language::Python => match node_kind {
            "function_definition" if parent_kind == Some("class_definition") => Some("method"),
            "function_definition" => Some("function"),
            "class_definition" => Some("class"),
            "import_statement" | "import_from_statement" => Some("import"),
            "expression_statement" if parent_kind == Some("module") => Some("declaration"),
            _ => None,
        },
    }
}

pub fn is_container(kind: &str) -> bool {
    matches!(
        kind,
        "class" | "struct" | "enum" | "trait" | "impl" | "interface" | "module"
    )
}
