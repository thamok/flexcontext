use std::fmt::Write;

use crate::model::SearchResponse;

pub fn render_human(response: &SearchResponse) -> String {
    let mut output = String::new();
    for (index, result) in response.results.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        let _ = writeln!(output, "{:.2}  {}", result.score, result.path);
        let container = result
            .containing_symbol
            .as_deref()
            .map(|name| format!(" in {name}"))
            .unwrap_or_default();
        let _ = writeln!(
            output,
            "{} {}{}\nlines {}–{} · {} bytes{}\n",
            result.kind,
            result.symbol,
            container,
            result.start_line,
            result.end_line,
            result.content_bytes,
            if result.content_truncated {
                " · compacted"
            } else {
                ""
            }
        );
        output.push_str(&result.content);
        if !result.content.ends_with('\n') {
            output.push('\n');
        }
        if !result.relations.is_empty() {
            output.push_str("\nrelated:\n");
            for relation in result.relations.iter().take(12) {
                let _ = writeln!(
                    output,
                    "  {:<12} {:<28} {}:{}",
                    relation.kind, relation.symbol, relation.path, relation.start_line
                );
            }
        }
    }
    if response.results.is_empty() {
        output.push_str("No structurally relevant source units found.\n");
    }
    let stats = &response.stats;
    let _ = writeln!(
        output,
        "\n{} results · {} source bytes (~{} tokens) · {} payload bytes · {} files indexed · {} symbols · {} µs",
        stats.returned_symbols,
        stats.returned_bytes,
        stats.approximate_tokens,
        stats.human_payload_bytes,
        stats.files_indexed,
        stats.symbols,
        stats.elapsed_us
    );
    output
}
