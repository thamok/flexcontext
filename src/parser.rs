use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result, anyhow};
use tree_sitter::{Node, Parser};

use crate::language::{configure_parser, structural_kind};
use crate::lexical::normalize_identifier;
use crate::model::{Language, SourceFile, Symbol};

pub fn extract_symbols(file: &SourceFile, next_id: &mut usize) -> Result<Vec<Symbol>> {
    SymbolExtractor::new().extract(file, next_id)
}

pub struct SymbolExtractor {
    parsers: HashMap<Language, Parser>,
}

impl SymbolExtractor {
    pub fn new() -> Self {
        Self {
            parsers: HashMap::new(),
        }
    }

    pub fn extract(&mut self, file: &SourceFile, next_id: &mut usize) -> Result<Vec<Symbol>> {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.parsers.entry(file.language)
        {
            let mut parser = Parser::new();
            configure_parser(&mut parser, file.language)
                .map_err(|error| anyhow!("failed to load {:?} grammar: {error}", file.language))?;
            entry.insert(parser);
        }
        let tree = self
            .parsers
            .get_mut(&file.language)
            .expect("parser was inserted")
            .parse(&file.source, None)
            .ok_or_else(|| anyhow!("Tree-sitter cancelled parsing {}", file.relative_path))?;
        let root = tree.root_node();
        let imports = collect_imports(root, file);
        let mut symbols = Vec::new();
        collect_units(root, file, &imports, next_id, &mut symbols)?;
        Ok(symbols)
    }
}

impl Default for SymbolExtractor {
    fn default() -> Self {
        Self::new()
    }
}

fn collect_units(
    node: Node<'_>,
    file: &SourceFile,
    imports: &[String],
    next_id: &mut usize,
    symbols: &mut Vec<Symbol>,
) -> Result<()> {
    let containing_node = nearest_structural_ancestor(node, file.language);
    let parent_kind = containing_node.map(|ancestor| ancestor.kind());
    if let Some(kind) = structural_kind(file.language, node.kind(), parent_kind)
        && let Some(symbol) = build_symbol(node, kind, containing_node, file, imports, *next_id)?
    {
        *next_id += 1;
        symbols.push(symbol);
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_units(child, file, imports, next_id, symbols)?;
    }
    Ok(())
}

fn build_symbol(
    node: Node<'_>,
    kind: &str,
    containing_node: Option<Node<'_>>,
    file: &SourceFile,
    imports: &[String],
    id: usize,
) -> Result<Option<Symbol>> {
    let content_node = if node
        .parent()
        .is_some_and(|parent| parent.kind() == "decorated_definition")
    {
        node.parent().unwrap_or(node)
    } else {
        node
    };
    let name = symbol_name(node, kind, &file.source);
    if name.is_empty() {
        return Ok(None);
    }
    let containing_symbol = containing_node
        .map(|ancestor| symbol_name(ancestor, "container", &file.source))
        .filter(|name| !name.is_empty());
    let mut comments = attached_comments(content_node, &file.source);
    if let Some(docstring) = python_docstring(node, file.language, &file.source) {
        if !comments.is_empty() {
            comments.push('\n');
        }
        comments.push_str(docstring);
    }
    let content_start =
        comment_start_byte(content_node, &file.source).unwrap_or(content_node.start_byte());
    let content = source_slice(&file.source, content_start, content_node.end_byte())?.to_owned();
    let body_node = node.child_by_field_name("body");
    let signature_end = body_node.map_or(node.end_byte(), |body| body.start_byte());
    let signature = source_slice(&file.source, node.start_byte(), signature_end)?
        .trim()
        .to_owned();
    let body = body_node
        .map(|body| source_slice(&file.source, body.start_byte(), body.end_byte()))
        .transpose()?
        .unwrap_or("")
        .to_owned();
    let identifiers = collect_identifiers(node, &file.source);
    let type_references = collect_type_references(node, &file.source);
    let calls = collect_calls(node, file.language, &file.source);

    Ok(Some(Symbol {
        id,
        path: file.relative_path.clone(),
        language: file.language,
        normalized_name: normalize_identifier(&name),
        name,
        kind: kind.to_owned(),
        containing_symbol,
        structural_depth: structural_depth(node, file.language),
        start_byte: content_start,
        end_byte: content_node.end_byte(),
        start_line: byte_line(&file.source, content_start),
        end_line: content_node.end_position().row + 1,
        signature,
        body,
        comments,
        content,
        imports: imports.to_vec(),
        identifiers,
        type_references,
        calls,
    }))
}

fn nearest_structural_ancestor(node: Node<'_>, language: Language) -> Option<Node<'_>> {
    let mut parent = node.parent();
    while let Some(ancestor) = parent {
        if structural_kind(language, ancestor.kind(), None).is_some() {
            return Some(ancestor);
        }
        parent = ancestor.parent();
    }
    None
}

fn symbol_name(node: Node<'_>, kind: &str, source: &str) -> String {
    for field in ["name", "type", "declarator"] {
        if let Some(name) = node.child_by_field_name(field) {
            if let Some(identifier) = first_identifier(name, source) {
                return identifier;
            }
            if let Ok(text) = name.utf8_text(source.as_bytes()) {
                let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !compact.is_empty() && compact.len() <= 120 {
                    return compact;
                }
            }
        }
    }
    if kind == "import" {
        return import_name(node, source);
    }
    first_identifier(node, source).unwrap_or_default()
}

fn import_name(node: Node<'_>, source: &str) -> String {
    if node.kind() == "import_statement"
        && let Some(identifier) = first_identifier(node, source)
    {
        return identifier;
    }
    let text = node.utf8_text(source.as_bytes()).unwrap_or("import").trim();
    let candidate = text
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-'))
        .filter(|part| !part.is_empty())
        .find(|part| !matches!(*part, "use" | "import" | "from" | "as" | "require"))
        .unwrap_or("import");
    candidate.to_owned()
}

fn first_identifier(node: Node<'_>, source: &str) -> Option<String> {
    if is_identifier_kind(node.kind()) {
        return node
            .utf8_text(source.as_bytes())
            .ok()
            .map(ToOwned::to_owned);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(identifier) = first_identifier(child, source) {
            return Some(identifier);
        }
    }
    None
}

fn collect_identifiers(node: Node<'_>, source: &str) -> Vec<String> {
    let mut identifiers = BTreeSet::new();
    visit(node, &mut |child| {
        if is_identifier_kind(child.kind())
            && let Ok(text) = child.utf8_text(source.as_bytes())
            && text.len() <= 160
        {
            identifiers.insert(text.to_owned());
        }
    });
    identifiers.into_iter().collect()
}

fn collect_type_references(node: Node<'_>, source: &str) -> Vec<String> {
    let mut identifiers = BTreeSet::new();
    visit(node, &mut |child| {
        if matches!(child.kind(), "type_identifier" | "namespace_identifier")
            && let Ok(text) = child.utf8_text(source.as_bytes())
            && text.len() <= 160
        {
            identifiers.insert(text.to_owned());
        }
    });
    identifiers.into_iter().collect()
}

fn structural_depth(node: Node<'_>, language: Language) -> usize {
    let mut depth = 0;
    let mut parent = node.parent();
    while let Some(ancestor) = parent {
        if structural_kind(language, ancestor.kind(), None).is_some() {
            depth += 1;
        }
        parent = ancestor.parent();
    }
    depth
}

fn collect_calls(node: Node<'_>, language: Language, source: &str) -> Vec<String> {
    let mut calls = BTreeSet::new();
    visit(node, &mut |child| {
        let target = match child.kind() {
            "call_expression" => child
                .child_by_field_name("function")
                .or_else(|| child.named_child(0)),
            "macro_invocation" if language == Language::Rust => child
                .child_by_field_name("macro")
                .or_else(|| child.named_child(0)),
            _ => None,
        };
        if let Some(target) = target
            && let Some(identifier) = last_identifier(target, source)
        {
            calls.insert(identifier);
        }
    });
    calls.into_iter().collect()
}

fn last_identifier(node: Node<'_>, source: &str) -> Option<String> {
    let mut found = None;
    visit(node, &mut |child| {
        if is_identifier_kind(child.kind())
            && let Ok(text) = child.utf8_text(source.as_bytes())
        {
            found = Some(text.to_owned());
        }
    });
    found
}

fn visit(node: Node<'_>, callback: &mut impl FnMut(Node<'_>)) {
    callback(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, callback);
    }
}

fn is_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "shorthand_property_identifier_pattern"
            | "namespace_identifier"
    )
}

fn collect_imports(root: Node<'_>, file: &SourceFile) -> Vec<String> {
    let mut imports = Vec::new();
    visit(root, &mut |node| {
        if structural_kind(file.language, node.kind(), None) == Some("import")
            && let Ok(text) = node.utf8_text(file.source.as_bytes())
        {
            imports.push(text.trim().to_owned());
        }
    });
    imports.sort();
    imports.dedup();
    imports
}

fn attached_comments(node: Node<'_>, source: &str) -> String {
    let Some(start) = comment_start_byte(node, source) else {
        return String::new();
    };
    source_slice(source, start, node.start_byte())
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn python_docstring<'a>(node: Node<'_>, language: Language, source: &'a str) -> Option<&'a str> {
    if language != Language::Python {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    let statement = body.named_child(0)?;
    if statement.kind() != "expression_statement" {
        return None;
    }
    let string = statement.named_child(0)?;
    if string.kind() != "string" {
        return None;
    }
    string.utf8_text(source.as_bytes()).ok()
}

fn comment_start_byte(node: Node<'_>, source: &str) -> Option<usize> {
    let mut sibling = node.prev_named_sibling();
    let mut start = None;
    let mut next_start_line = node.start_position().row;
    while let Some(previous) = sibling {
        if !is_comment_kind(previous.kind())
            || next_start_line.saturating_sub(previous.end_position().row) > 2
        {
            break;
        }
        if source_slice(source, previous.end_byte(), node.start_byte())
            .is_ok_and(|gap| gap.lines().any(|line| !line.trim().is_empty()))
        {
            break;
        }
        start = Some(previous.start_byte());
        next_start_line = previous.start_position().row;
        sibling = previous.prev_named_sibling();
    }
    start
}

fn is_comment_kind(kind: &str) -> bool {
    kind == "comment" || kind.contains("comment")
}

fn byte_line(source: &str, byte: usize) -> usize {
    source.as_bytes()[..byte.min(source.len())]
        .iter()
        .filter(|&&value| value == b'\n')
        .count()
        + 1
}

fn source_slice(source: &str, start: usize, end: usize) -> Result<&str> {
    source
        .get(start..end)
        .with_context(|| format!("invalid Tree-sitter byte range {start}..{end}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn file(language: Language, source: &str) -> SourceFile {
        SourceFile {
            absolute_path: PathBuf::from("test"),
            relative_path: "test".to_owned(),
            language,
            source: source.to_owned(),
        }
    }

    #[test]
    fn extracts_rust_nested_structures_and_calls() {
        let source = "/// A session\nstruct Session;\nimpl Session { fn authenticate_user(&self) { validate_token(); } }";
        let symbols = extract_symbols(&file(Language::Rust, source), &mut 0).unwrap();
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "Session" && symbol.kind == "struct")
        );
        let method = symbols
            .iter()
            .find(|symbol| symbol.name == "authenticate_user")
            .unwrap();
        assert_eq!(method.kind, "method");
        assert_eq!(method.containing_symbol.as_deref(), Some("Session"));
        assert!(method.calls.contains(&"validate_token".to_owned()));
    }

    #[test]
    fn extracts_typescript_and_python() {
        let ts = extract_symbols(
            &file(
                Language::TypeScript,
                "export class AuthService { validateToken(token: string) { return token; } }",
            ),
            &mut 0,
        )
        .unwrap();
        assert!(
            ts.iter()
                .any(|symbol| symbol.name == "validateToken" && symbol.kind == "method")
        );
        let py = extract_symbols(
            &file(
                Language::Python,
                "class UserSession:\n    def authenticate_user(self):\n        validate_token()\n",
            ),
            &mut 0,
        )
        .unwrap();
        assert!(
            py.iter()
                .any(|symbol| symbol.name == "authenticate_user" && symbol.kind == "method")
        );
    }
}
