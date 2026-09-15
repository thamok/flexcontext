use std::collections::{BTreeSet, HashMap};

use crate::lexical::normalize_identifier;
use crate::model::{Relation, ScoredSymbol, Symbol};
use crate::ranking::sort_scored;

#[derive(Debug, Clone)]
pub struct ResolvedRelation {
    pub kind: &'static str,
    pub target_id: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RelationGraph {
    pub outgoing: HashMap<usize, Vec<ResolvedRelation>>,
}

pub fn build_relation_graph(symbols: &[Symbol]) -> RelationGraph {
    let sources: Vec<_> = symbols.iter().map(|symbol| symbol.id).collect();
    build_relation_graph_for(symbols, &sources)
}

pub fn build_relation_graph_for(symbols: &[Symbol], source_ids: &[usize]) -> RelationGraph {
    RelationIndex::build(symbols).graph_for(symbols, source_ids)
}

/// Stable name/member lookup tables retained alongside a resident snapshot.
#[derive(Debug)]
pub struct RelationIndex {
    definitions: HashMap<String, Vec<usize>>,
    members: HashMap<(String, String), Vec<usize>>,
    imports: HashMap<String, Vec<usize>>,
}
impl RelationIndex {
    pub fn build(symbols: &[Symbol]) -> Self {
        let mut definitions: HashMap<String, Vec<usize>> = HashMap::new();
        for symbol in symbols.iter().filter(|symbol| symbol.kind != "import") {
            definitions
                .entry(symbol.normalized_name.clone())
                .or_default()
                .push(symbol.id);
        }

        let mut members: HashMap<(String, String), Vec<usize>> = HashMap::new();
        let mut imports: HashMap<String, Vec<usize>> = HashMap::new();
        for symbol in symbols {
            if let Some(container) = &symbol.containing_symbol {
                members
                    .entry((symbol.path.clone(), normalize_identifier(container)))
                    .or_default()
                    .push(symbol.id);
            }
            if symbol.kind == "import" {
                imports
                    .entry(symbol.path.clone())
                    .or_default()
                    .push(symbol.id);
            }
        }

        Self {
            definitions,
            members,
            imports,
        }
    }
    pub fn graph_for(&self, symbols: &[Symbol], source_ids: &[usize]) -> RelationGraph {
        let definitions = &self.definitions;
        let members = &self.members;
        let imports = &self.imports;
        let mut graph = RelationGraph::default();
        for &source_id in source_ids {
            let Some(symbol) = symbols.get(source_id) else {
                continue;
            };
            let mut seen = BTreeSet::new();
            if let Some(container) = &symbol.containing_symbol {
                connect_name(
                    &mut graph,
                    &mut seen,
                    symbol,
                    "enclosed_by",
                    container,
                    definitions,
                    symbols,
                );
            }
            for called in &symbol.calls {
                connect_name(
                    &mut graph,
                    &mut seen,
                    symbol,
                    "calls",
                    called,
                    definitions,
                    symbols,
                );
            }
            for identifier in &symbol.type_references {
                if identifier == &symbol.name {
                    continue;
                }
                connect_name(
                    &mut graph,
                    &mut seen,
                    symbol,
                    "type_reference",
                    identifier,
                    definitions,
                    symbols,
                );
            }
            if symbol.kind != "import" {
                for &import_id in imports
                    .get(symbol.path.as_str())
                    .into_iter()
                    .flatten()
                    .take(8)
                {
                    add_edge(&mut graph, &mut seen, symbol.id, import_id, "imports");
                }
            }
            if crate::language::is_container(&symbol.kind) {
                let key = (symbol.path.clone(), symbol.normalized_name.clone());
                for &member in members.get(&key).into_iter().flatten() {
                    if member != symbol.id {
                        add_edge(&mut graph, &mut seen, symbol.id, member, "contains");
                    }
                }
            }
        }
        for relations in graph.outgoing.values_mut() {
            relations.sort_by_key(|relation| (relation.kind, relation.target_id));
        }
        graph
    }
}

pub fn expand_ranked(
    mut ranked: Vec<ScoredSymbol>,
    graph: &RelationGraph,
    symbols: &[Symbol],
) -> Vec<ScoredSymbol> {
    let seeds: Vec<_> = ranked.iter().take(8).cloned().collect();
    let mut positions: HashMap<usize, usize> = ranked
        .iter()
        .enumerate()
        .map(|(index, item)| (item.symbol_id, index))
        .collect();
    for seed in seeds {
        let Some(relations) = graph.outgoing.get(&seed.symbol_id) else {
            continue;
        };
        for relation in relations {
            let boost = match relation.kind {
                "calls" => 4.0,
                "enclosed_by" => 3.0,
                "type_reference" => 2.5,
                "contains" => 2.0,
                "imports" => 0.75,
                _ => 0.0,
            };
            if let Some(&index) = positions.get(&relation.target_id) {
                let item = &mut ranked[index];
                item.signals.structural_relation =
                    (item.signals.structural_relation + boost).min(6.0);
                item.score = item.signals.total();
            } else {
                let mut item = ScoredSymbol {
                    symbol_id: relation.target_id,
                    score: boost,
                    signals: Default::default(),
                };
                item.signals.structural_relation = boost;
                positions.insert(relation.target_id, ranked.len());
                ranked.push(item);
            }
        }
    }
    sort_scored(&mut ranked, symbols);
    ranked
}

pub fn serializable_relations(
    symbol_id: usize,
    graph: &RelationGraph,
    symbols: &[Symbol],
) -> Vec<Relation> {
    graph
        .outgoing
        .get(&symbol_id)
        .into_iter()
        .flatten()
        .map(|relation| {
            let target = &symbols[relation.target_id];
            Relation {
                kind: relation.kind.to_owned(),
                symbol: target.name.clone(),
                path: target.path.clone(),
                start_line: target.start_line,
            }
        })
        .collect()
}

fn connect_name(
    graph: &mut RelationGraph,
    seen: &mut BTreeSet<(usize, &'static str)>,
    source: &Symbol,
    kind: &'static str,
    name: &str,
    definitions: &HashMap<String, Vec<usize>>,
    symbols: &[Symbol],
) {
    let normalized = normalize_identifier(name);
    let Some(matches) = definitions.get(normalized.as_str()) else {
        return;
    };
    let same_file: Vec<_> = matches
        .iter()
        .copied()
        .filter(|&id| symbols[id].path == source.path && id != source.id)
        .collect();
    if same_file.len() == 1 {
        add_edge(graph, seen, source.id, same_file[0], kind);
        return;
    }
    if kind == "enclosed_by" || name == "require" {
        return;
    }
    let other_matches: Vec<_> = matches
        .iter()
        .copied()
        .filter(|&id| {
            id != source.id
                && symbols[id].structural_depth == 0
                && (kind != "type_reference"
                    || matches!(
                        symbols[id].kind.as_str(),
                        "type" | "interface" | "class" | "struct" | "enum" | "trait"
                    ))
        })
        .collect();
    if other_matches.len() == 1 {
        add_edge(graph, seen, source.id, other_matches[0], kind);
    }
}

fn add_edge(
    graph: &mut RelationGraph,
    seen: &mut BTreeSet<(usize, &'static str)>,
    source: usize,
    target: usize,
    kind: &'static str,
) {
    if seen.insert((target, kind)) {
        graph
            .outgoing
            .entry(source)
            .or_default()
            .push(ResolvedRelation {
                kind,
                target_id: target,
            });
    }
}
