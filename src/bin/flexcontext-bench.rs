use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use flexcontext::{SearchOptions, search};
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(about = "Reproducible flexcontext versus ripgrep context comparison")]
struct Args {
    #[arg(default_value = ".")]
    root: PathBuf,

    #[arg(long, default_value_t = 16_384)]
    max_bytes: usize,

    #[arg(
        long,
        value_delimiter = ',',
        default_value = "auth,authentication,validate token,UserSession,config,parseRequest"
    )]
    queries: Vec<String>,

    #[arg(long)]
    json: bool,

    /// Optional relevance judgments used to report precision@k, recall@k, and relevant-byte ratio.
    #[arg(long)]
    quality_file: Option<PathBuf>,

    #[arg(long, default_value_t = 5)]
    quality_k: usize,
}

#[derive(Debug, Serialize)]
struct Comparison {
    query: String,
    rg_output_bytes: usize,
    flexcontext_output_bytes: usize,
    flexcontext_payload_bytes: usize,
    rg_latency_us: u128,
    flexcontext_latency_us: u128,
    flexcontext_symbols: usize,
    flexcontext_candidates: usize,
    cache_reused: bool,
}

#[derive(Debug, Deserialize)]
struct RelevanceCase {
    query: String,
    relevant_symbols: Vec<String>,
}

#[derive(Debug, Serialize)]
struct QualityMeasurement {
    query: String,
    k: usize,
    precision_at_k: f64,
    recall_at_k: f64,
    relevant_bytes_ratio: f64,
    returned_bytes: usize,
}

#[derive(Debug, Serialize)]
struct Report {
    comparisons: Vec<Comparison>,
    quality: Vec<QualityMeasurement>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut comparisons = Vec::new();
    for query in &args.queries {
        let started = Instant::now();
        let rg = Command::new("rg")
            .args(["--line-number", "--color", "never", "--", query])
            .arg(&args.root)
            .output()
            .context("failed to run rg; install ripgrep to use the comparison benchmark")?;
        let rg_latency_us = started.elapsed().as_micros();
        if !rg.status.success() && rg.status.code() != Some(1) {
            anyhow::bail!("rg failed: {}", String::from_utf8_lossy(&rg.stderr));
        }

        let started = Instant::now();
        let response = search(&SearchOptions {
            root: args.root.clone(),
            query: query.clone(),
            max_bytes: args.max_bytes,
            max_results: 12,
            use_cache: true,
        })?;
        let flexcontext_latency_us = started.elapsed().as_micros();
        comparisons.push(Comparison {
            query: query.clone(),
            rg_output_bytes: rg.stdout.len(),
            flexcontext_output_bytes: response.stats.returned_bytes,
            flexcontext_payload_bytes: response.stats.human_payload_bytes,
            rg_latency_us,
            flexcontext_latency_us,
            flexcontext_symbols: response.stats.returned_symbols,
            flexcontext_candidates: response.stats.candidate_symbols,
            cache_reused: response.stats.index_reused,
        });
    }
    let quality = if let Some(path) = &args.quality_file {
        measure_quality(&args, path)?
    } else {
        Vec::new()
    };
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&Report {
                comparisons,
                quality
            })?
        );
    } else {
        println!(
            "{:<20} {:>10} {:>10} {:>12} {:>10} {:>10} {:>8} {:>7}",
            "query", "rg bytes", "source", "payload", "rg µs", "flex µs", "symbols", "cached"
        );
        for row in comparisons {
            println!(
                "{:<20} {:>10} {:>10} {:>12} {:>10} {:>10} {:>8} {:>7}",
                row.query,
                row.rg_output_bytes,
                row.flexcontext_output_bytes,
                row.flexcontext_payload_bytes,
                row.rg_latency_us,
                row.flexcontext_latency_us,
                row.flexcontext_symbols,
                row.cache_reused
            );
        }
        if !quality.is_empty() {
            println!(
                "\n{:<20} {:>12} {:>12} {:>16} {:>10}",
                "quality query", "precision@k", "recall@k", "relevant bytes", "bytes"
            );
            for row in quality {
                println!(
                    "{:<20} {:>12.3} {:>12.3} {:>16.3} {:>10}",
                    row.query,
                    row.precision_at_k,
                    row.recall_at_k,
                    row.relevant_bytes_ratio,
                    row.returned_bytes
                );
            }
        }
    }
    Ok(())
}

fn measure_quality(args: &Args, path: &PathBuf) -> Result<Vec<QualityMeasurement>> {
    let cases: Vec<RelevanceCase> = serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?,
    )
    .with_context(|| format!("invalid relevance judgments in {}", path.display()))?;
    let quality_root = path.parent().unwrap_or(&args.root);
    cases
        .into_iter()
        .map(|case| {
            let response = search(&SearchOptions {
                root: quality_root.to_owned(),
                query: case.query.clone(),
                max_bytes: args.max_bytes,
                max_results: args.quality_k,
                use_cache: true,
            })?;
            let top = &response.results[..response.results.len().min(args.quality_k)];
            let is_relevant =
                |symbol: &str| case.relevant_symbols.iter().any(|name| name == symbol);
            let hits = top
                .iter()
                .filter(|result| is_relevant(&result.symbol))
                .count();
            let relevant_bytes: usize = top
                .iter()
                .filter(|result| is_relevant(&result.symbol))
                .map(|result| result.content_bytes)
                .sum();
            let returned_bytes: usize = top.iter().map(|result| result.content_bytes).sum();
            Ok(QualityMeasurement {
                query: case.query,
                k: args.quality_k,
                precision_at_k: hits as f64 / args.quality_k.max(1) as f64,
                recall_at_k: hits as f64 / case.relevant_symbols.len().max(1) as f64,
                relevant_bytes_ratio: relevant_bytes as f64 / returned_bytes.max(1) as f64,
                returned_bytes,
            })
        })
        .collect()
}
