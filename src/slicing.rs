//! Budgeted excerpts preserve complete syntax nodes and explicit original-file ranges.
use crate::{
    lexical::{Query, matching_query_terms},
    model::{SourceSpan, Symbol},
};
use tree_sitter::Node;

pub fn slice_symbol(
    symbol: &Symbol,
    query: &Query,
    limit: usize,
) -> Option<(String, Vec<SourceSpan>)> {
    let mut parser = tree_sitter::Parser::new();
    crate::language::configure_parser(&mut parser, symbol.language).ok()?;
    let tree = parser.parse(&symbol.content, None)?;
    let mut content = format!(
        "{}\n[… body excerpts; omitted code is not shown …]\n",
        symbol.signature
    );
    if content.len() > limit {
        return None;
    }
    let mut candidates = Vec::new();
    collect(
        tree.root_node(),
        &symbol.content,
        limit.saturating_sub(content.len()),
        &mut candidates,
    );
    candidates.sort_by(|a, b| {
        let relevance =
            |node: &Node<'_>| matching_query_terms(query, &symbol.content[node.byte_range()]);
        relevance(b)
            .cmp(&relevance(a))
            .then_with(|| a.start_byte().cmp(&b.start_byte()))
    });
    let mut chosen = Vec::new();
    let mut used = content.len();
    for node in candidates {
        let header = format!(
            "\n[lines {}–{}]\n",
            symbol.start_line + node.start_position().row,
            symbol.start_line + node.end_position().row
        );
        let cost = header.len() + node.byte_range().len() + 1;
        if cost + used <= limit {
            chosen.push(node);
            used += cost;
        }
    }
    chosen.sort_by_key(Node::start_byte);
    let mut spans = Vec::new();
    for node in chosen {
        let start_line = symbol.start_line + node.start_position().row;
        let end_line = symbol.start_line + node.end_position().row;
        content.push_str(&format!("\n[lines {start_line}–{end_line}]\n"));
        content.push_str(&symbol.content[node.byte_range()]);
        content.push('\n');
        spans.push(SourceSpan {
            start_byte: symbol.start_byte + node.start_byte(),
            end_byte: symbol.start_byte + node.end_byte(),
            start_line,
            end_line,
        });
    }
    Some((content, spans))
}

fn collect<'a>(node: Node<'a>, source: &str, limit: usize, output: &mut Vec<Node<'a>>) {
    // A block's children are complete statements/branches. Descend into oversized
    // branches, closures and try blocks to find smaller complete units.
    let parent_is_block = node
        .parent()
        .is_some_and(|parent| matches!(parent.kind(), "statement_block" | "block"));
    if parent_is_block
        && node.kind() != "comment"
        && node.byte_range().len() + 48 <= limit
        && !node.has_error()
        && !source[node.byte_range()].trim().is_empty()
    {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, source, limit, output);
    }
}
