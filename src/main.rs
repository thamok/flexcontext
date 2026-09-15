use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::Parser;
use flexcontext::{SearchOptions, search};

#[derive(Debug, Parser)]
#[command(
    name = "flexcontext",
    version,
    about = "Structural lexical context retrieval for coding agents"
)]
struct Cli {
    /// Search ROOT for structurally relevant code matching QUERY.
    #[arg(long, value_names = ["ROOT", "QUERY"], num_args = 2)]
    code_search: Vec<String>,

    /// Serve a resident repository snapshot over MCP stdio.
    #[arg(long, value_name = "ROOT", conflicts_with_all = ["code_search", "json", "max_bytes", "budget", "max_results"])]
    mcp: Option<PathBuf>,

    /// Approximate source-token budget (four bytes per token).
    #[arg(long)]
    budget: Option<usize>,

    /// Emit an inspectable JSON response.
    #[arg(long)]
    json: bool,

    /// Maximum UTF-8 source bytes returned across all structures.
    #[arg(long)]
    max_bytes: Option<usize>,

    /// Maximum number of structural units returned.
    #[arg(long, default_value_t = 12)]
    max_results: usize,

    /// Parse every file and do not read or update the repository-local index cache.
    #[arg(long)]
    no_cache: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(root) = &cli.mcp {
        let mut session = flexcontext::SearchSession::open(root, !cli.no_cache)?;
        return flexcontext::mcp::serve(
            &mut session,
            std::io::stdin().lock(),
            std::io::stdout().lock(),
        );
    }
    let max_bytes = match cli.budget {
        Some(0) => return Err(anyhow!("--budget must be greater than zero")),
        Some(tokens) => tokens
            .checked_mul(4)
            .ok_or_else(|| anyhow!("budget is too large"))?
            .min(cli.max_bytes.unwrap_or(usize::MAX)),
        None => cli.max_bytes.unwrap_or(16_384),
    };
    let [root, query] = cli.code_search.as_slice() else {
        return Err(anyhow!("use --code-search <ROOT> <QUERY>"));
    };
    let response = search(&SearchOptions {
        root: PathBuf::from(root),
        query: query.clone(),
        max_bytes,
        max_results: cli.max_results,
        use_cache: !cli.no_cache,
    })?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print!("{}", flexcontext::output::render_human(&response));
    }
    Ok(())
}
