use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use colored::Colorize;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

mod cmd_build;
mod config;
mod install;
mod skill;

#[derive(Parser)]
#[command(
    name = "graphify-rs",
    version,
    about = "AI-powered knowledge graph builder"
)]
struct Cli {
    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Enable verbose output (debug-level)
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Number of parallel jobs (default: number of CPUs)
    #[arg(short, long, global = true)]
    jobs: Option<usize>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build knowledge graph from files in a directory
    Build {
        #[arg(short, long, default_value = ".")]
        path: String,
        #[arg(short, long, default_value = "graphify-out")]
        output: String,
        #[arg(long)]
        no_llm: bool,
        #[arg(long)]
        code_only: bool,
        /// Only re-extract new/modified files since last build
        #[arg(long)]
        update: bool,
        /// Export formats (comma-separated). Available: json,html,graphml,cypher,svg,wiki,obsidian,report. Default: all
        #[arg(long, value_delimiter = ',')]
        format: Vec<String>,
        /// Maximum nodes in HTML visualization (default: 2000). Larger values may slow browser.
        #[arg(long)]
        max_viz_nodes: Option<usize>,
    },
    /// Install graphify skill for AI coding assistant
    Install {
        #[arg(long, default_value = "claude")]
        platform: String,
    },
    /// Query the knowledge graph
    Query {
        question: String,
        #[arg(long)]
        dfs: bool,
        #[arg(long, default_value_t = 2000)]
        budget: usize,
        #[arg(long, default_value = "graphify-out/graph.json")]
        graph: String,
    },
    /// Run benchmark
    Benchmark {
        #[arg(default_value = "graphify-out/graph.json")]
        graph_path: String,
    },
    /// Git hook management
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
    /// Claude Code integration
    Claude {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// CodeBuddy integration
    Codebuddy {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// Codex integration
    Codex {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// OpenCode integration
    Opencode {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// OpenClaw integration
    Claw {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// Factory Droid integration
    Droid {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// Trae integration
    Trae {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// Trae CN integration
    TraeCn {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// Save query result to memory
    SaveResult {
        #[arg(long)]
        question: String,
        #[arg(long)]
        answer: String,
        #[arg(long, default_value = "query")]
        r#type: String,
        #[arg(long)]
        nodes: Vec<String>,
        #[arg(long, default_value = "graphify-out/memory")]
        memory_dir: String,
    },
    /// Start MCP server
    Serve {
        #[arg(long, default_value = "graphify-out/graph.json")]
        graph: String,
    },
    /// Watch for file changes and rebuild
    Watch {
        #[arg(short, long, default_value = ".")]
        path: String,
        #[arg(short, long, default_value = "graphify-out")]
        output: String,
    },
    /// Ingest URL content
    Ingest {
        url: String,
        #[arg(short, long, default_value = "graphify-out")]
        output: String,
    },
    /// Compare two graph snapshots
    Diff {
        /// Path to the old graph.json
        old: String,
        /// Path to the new graph.json
        new: String,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        output: String,
    },
    /// Show graph statistics without rebuilding
    Stats {
        /// Path to graph.json
        #[arg(default_value = "graphify-out/graph.json")]
        graph: String,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },
    /// Initialize a graphify.toml config file
    Init,
}

#[derive(Subcommand)]
enum HookAction {
    /// Install git hooks
    Install,
    /// Uninstall git hooks
    Uninstall,
    /// Show hook status
    Status,
}

#[derive(Subcommand)]
enum PlatformAction {
    /// Install platform integration
    Install,
    /// Uninstall platform integration
    Uninstall,
}

/// Verbosity level derived from --quiet / --verbose flags.
#[derive(Clone, Copy)]
pub(crate) enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

impl Verbosity {
    pub(crate) fn from_flags(quiet: bool, verbose: bool) -> Self {
        if quiet {
            Self::Quiet
        } else if verbose {
            Self::Verbose
        } else {
            Self::Normal
        }
    }

    pub(crate) fn is_quiet(self) -> bool {
        matches!(self, Self::Quiet)
    }

    pub(crate) fn is_verbose(self) -> bool {
        matches!(self, Self::Verbose)
    }
}

/// Print helper that respects verbosity.
#[macro_export]
macro_rules! info_print {
    ($verb:expr, $($arg:tt)*) => {
        if !$verb.is_quiet() {
            println!($($arg)*);
        }
    };
}

#[macro_export]
macro_rules! verbose_print {
    ($verb:expr, $($arg:tt)*) => {
        if $verb.is_verbose() {
            println!($($arg)*);
        }
    };
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    install::check_skill_versions();

    let verb = Verbosity::from_flags(cli.quiet, cli.verbose);

    let filter = if cli.verbose {
        "debug"
    } else if cli.quiet {
        "error"
    } else {
        &std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string())
    };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();

    if let Some(jobs) = cli.jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build_global()
            .ok(); // ignore if already initialized
    }

    match cli.command {
        Commands::Build {
            path,
            output,
            no_llm,
            code_only,
            update,
            format,
            max_viz_nodes,
        } => {
            let app_cfg = config::load_config(Path::new(&path));
            let effective_path = path;
            let effective_output = if output == "graphify-out" {
                app_cfg.output.unwrap_or(output)
            } else {
                output
            };
            let effective_no_llm = no_llm || app_cfg.no_llm.unwrap_or(false);
            let effective_code_only = code_only || app_cfg.code_only.unwrap_or(false);
            let effective_formats = if format.is_empty() {
                app_cfg.formats.unwrap_or_default()
            } else {
                format
            };

            cmd_build::cmd_build(
                &effective_path,
                &effective_output,
                effective_no_llm,
                effective_code_only,
                update,
                &effective_formats,
                verb,
                cli.jobs,
                max_viz_nodes,
                app_cfg.llm,
            )
            .await?;
        }
        Commands::Install { platform } => {
            install::install_skill(&platform)?;
        }
        Commands::Query {
            question,
            dfs,
            budget,
            graph,
        } => {
            cmd_query(&question, dfs, budget, &graph)?;
        }
        Commands::Benchmark { graph_path } => {
            let result = graphify_benchmark::run_benchmark(Path::new(&graph_path), None)?;
            graphify_benchmark::print_benchmark(&result);
        }
        Commands::Hook { action } => {
            let root = Path::new(".");
            match action {
                HookAction::Install => println!("{}", graphify_hooks::install_hooks(root)?),
                HookAction::Uninstall => println!("{}", graphify_hooks::uninstall_hooks(root)?),
                HookAction::Status => println!("{}", graphify_hooks::hook_status(root)?),
            }
        }
        Commands::Claude { action } => {
            let root = Path::new(".");
            match action {
                PlatformAction::Install => install::claude_install(root)?,
                PlatformAction::Uninstall => install::claude_uninstall(root)?,
            }
        }
        Commands::Codebuddy { action } => {
            let root = Path::new(".");
            match action {
                PlatformAction::Install => install::codebuddy_install(root)?,
                PlatformAction::Uninstall => install::codebuddy_uninstall(root)?,
            }
        }
        Commands::Codex { action } => {
            let root = Path::new(".");
            match action {
                PlatformAction::Install => install::codex_install(root)?,
                PlatformAction::Uninstall => install::codex_uninstall(root)?,
            }
        }
        Commands::Opencode { action } => {
            let root = Path::new(".");
            match action {
                PlatformAction::Install => install::opencode_install(root)?,
                PlatformAction::Uninstall => install::opencode_uninstall(root)?,
            }
        }
        Commands::Claw { action } => {
            let root = Path::new(".");
            match action {
                PlatformAction::Install => install::generic_platform_install(root, "Claw")?,
                PlatformAction::Uninstall => install::generic_platform_uninstall(root, "Claw")?,
            }
        }
        Commands::Droid { action } => {
            let root = Path::new(".");
            match action {
                PlatformAction::Install => install::generic_platform_install(root, "Droid")?,
                PlatformAction::Uninstall => install::generic_platform_uninstall(root, "Droid")?,
            }
        }
        Commands::Trae { action } => {
            let root = Path::new(".");
            match action {
                PlatformAction::Install => install::generic_platform_install(root, "Trae")?,
                PlatformAction::Uninstall => install::generic_platform_uninstall(root, "Trae")?,
            }
        }
        Commands::TraeCn { action } => {
            let root = Path::new(".");
            match action {
                PlatformAction::Install => install::generic_platform_install(root, "Trae CN")?,
                PlatformAction::Uninstall => install::generic_platform_uninstall(root, "Trae CN")?,
            }
        }
        Commands::SaveResult {
            question,
            answer,
            r#type,
            nodes,
            memory_dir,
        } => {
            let nodes_ref: Option<&[String]> = if nodes.is_empty() { None } else { Some(&nodes) };
            let out = graphify_ingest::save_query_result(
                &question,
                &answer,
                Path::new(&memory_dir),
                &r#type,
                nodes_ref,
            )?;
            println!("Saved to {}", out.display());
        }
        Commands::Serve { graph } => {
            let graph_path = Path::new(&graph);
            if !graph_path.exists() {
                tracing::info!("{} not found, running auto-build...", graph_path.display());
                let output_dir = graph_path
                    .parent()
                    .unwrap_or(Path::new("graphify-out"))
                    .to_string_lossy()
                    .to_string();
                cmd_build::cmd_build(
                    ".",
                    &output_dir,
                    true,
                    true,
                    false,
                    &["json".to_string()],
                    Verbosity::Quiet,
                    None,
                    None,
                    None,
                )
                .await
                .context("Auto-build failed")?;
            }
            graphify_serve::start_server(graph_path).await?;
        }
        Commands::Watch { path, output } => {
            graphify_watch::watch_directory(Path::new(&path), Path::new(&output)).await?;
        }
        Commands::Ingest { url, output } => {
            let out = graphify_ingest::ingest_url(&url, Path::new(&output)).await?;
            println!("Ingested to {}", out.display());
        }
        Commands::Diff { old, new, output } => {
            cmd_diff(&old, &new, &output)?;
        }
        Commands::Stats { graph } => {
            cmd_stats(&graph)?;
        }
        Commands::Completions { shell } => {
            generate(shell, &mut Cli::command(), "graphify-rs", &mut io::stdout());
        }
        Commands::Init => {
            cmd_init()?;
        }
    }

    Ok(())
}

/// Query the knowledge graph
fn cmd_query(question: &str, use_dfs: bool, budget: usize, graph_path: &str) -> Result<()> {
    let gp = PathBuf::from(graph_path);
    if !gp.exists() {
        anyhow::bail!("Graph file not found: {}", gp.display());
    }

    let json_str = std::fs::read_to_string(&gp).context("Could not read graph file")?;
    let json_value: serde_json::Value =
        serde_json::from_str(&json_str).context("Could not parse graph JSON")?;
    let graph = graphify_core::graph::KnowledgeGraph::from_node_link_json(&json_value)
        .context("Could not load graph from JSON")?;

    let terms: Vec<String> = question
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .map(str::to_lowercase)
        .collect();

    let scored = graphify_serve::score_nodes(&graph, &terms);
    if scored.is_empty() {
        println!("No matching nodes found.");
        return Ok(());
    }

    let start: Vec<String> = scored.iter().take(5).map(|(_, id)| id.clone()).collect();
    let (nodes, edges) = if use_dfs {
        graphify_serve::dfs(&graph, &start, 2)
    } else {
        graphify_serve::bfs(&graph, &start, 2)
    };
    let text = graphify_serve::subgraph_to_text(&graph, &nodes, &edges, budget);
    println!("{text}");

    Ok(())
}

/// Compare two graph snapshots and display differences
fn cmd_diff(old_path: &str, new_path: &str, output_format: &str) -> Result<()> {
    let old_p = PathBuf::from(old_path);
    let new_p = PathBuf::from(new_path);

    if !old_p.exists() {
        anyhow::bail!("Old graph file not found: {}", old_p.display());
    }
    if !new_p.exists() {
        anyhow::bail!("New graph file not found: {}", new_p.display());
    }

    let old_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&old_p).context("Could not read old graph file")?,
    )
    .context("Could not parse old graph JSON")?;
    let new_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&new_p).context("Could not read new graph file")?,
    )
    .context("Could not parse new graph JSON")?;

    let old_graph = graphify_core::graph::KnowledgeGraph::from_node_link_json(&old_json)
        .context("Could not load old graph")?;
    let new_graph = graphify_core::graph::KnowledgeGraph::from_node_link_json(&new_json)
        .context("Could not load new graph")?;

    let diff = graphify_analyze::graph_diff(&old_graph, &new_graph);

    if output_format == "json" {
        println!("{}", serde_json::to_string_pretty(&diff)?);
    } else {
        let added_nodes = diff.get("added_nodes").and_then(|v| v.as_array());
        let removed_nodes = diff.get("removed_nodes").and_then(|v| v.as_array());
        let added_edges = diff.get("added_edges").and_then(|v| v.as_array());
        let removed_edges = diff.get("removed_edges").and_then(|v| v.as_array());

        println!(
            "{} {} → {}",
            "Graph Diff:".bold(),
            old_p.display(),
            new_p.display()
        );
        println!("─────────────────────────────────────");

        if let Some(nodes) = added_nodes {
            println!("\n{} ({})", "+ Added nodes".green(), nodes.len());
            for n in nodes.iter().take(20) {
                println!("  {} {}", "+".green(), n.as_str().unwrap_or("?"));
            }
            if nodes.len() > 20 {
                println!("  ... and {} more", nodes.len() - 20);
            }
        }

        if let Some(nodes) = removed_nodes {
            println!("\n{} ({})", "- Removed nodes".red(), nodes.len());
            for n in nodes.iter().take(20) {
                println!("  {} {}", "-".red(), n.as_str().unwrap_or("?"));
            }
            if nodes.len() > 20 {
                println!("  ... and {} more", nodes.len() - 20);
            }
        }

        if let Some(edges) = added_edges {
            println!("\n{} ({})", "+ Added edges".green(), edges.len());
            for e in edges.iter().take(20) {
                if let Some(arr) = e.as_array() {
                    let parts: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                    println!(
                        "  {} {} --[{}]--> {}",
                        "+".green(),
                        parts.first().unwrap_or(&"?"),
                        parts.get(2).unwrap_or(&"?"),
                        parts.get(1).unwrap_or(&"?")
                    );
                }
            }
            if edges.len() > 20 {
                println!("  ... and {} more", edges.len() - 20);
            }
        }

        if let Some(edges) = removed_edges {
            println!("\n{} ({})", "- Removed edges".red(), edges.len());
            for e in edges.iter().take(20) {
                if let Some(arr) = e.as_array() {
                    let parts: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                    println!(
                        "  {} {} --[{}]--> {}",
                        "-".red(),
                        parts.first().unwrap_or(&"?"),
                        parts.get(2).unwrap_or(&"?"),
                        parts.get(1).unwrap_or(&"?")
                    );
                }
            }
            if edges.len() > 20 {
                println!("  ... and {} more", edges.len() - 20);
            }
        }

        let summary_added =
            added_nodes.map_or(0, std::vec::Vec::len) + added_edges.map_or(0, std::vec::Vec::len);
        let summary_removed = removed_nodes.map_or(0, std::vec::Vec::len)
            + removed_edges.map_or(0, std::vec::Vec::len);
        println!(
            "\n{}: {} additions, {} removals",
            "Summary".bold(),
            format!("+{summary_added}").green(),
            format!("-{summary_removed}").red()
        );
    }

    Ok(())
}

/// Show graph statistics without rebuilding
fn cmd_stats(graph_path: &str) -> Result<()> {
    let gp = PathBuf::from(graph_path);
    if !gp.exists() {
        anyhow::bail!("Graph file not found: {}", gp.display());
    }

    let json_str = std::fs::read_to_string(&gp).context("Could not read graph file")?;
    let json_value: serde_json::Value =
        serde_json::from_str(&json_str).context("Could not parse graph JSON")?;
    let graph = graphify_core::graph::KnowledgeGraph::from_node_link_json(&json_value)
        .context("Could not load graph from JSON")?;

    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    let mut type_counts: HashMap<String, usize> = HashMap::new();
    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(&id) {
            let type_name = format!("{:?}", node.node_type);
            *type_counts.entry(type_name).or_insert(0) += 1;
        }
    }

    let mut rel_counts: HashMap<String, usize> = HashMap::new();
    for edge in graph.edges() {
        *rel_counts.entry(edge.relation.clone()).or_insert(0) += 1;
    }

    let communities = graphify_cluster::cluster(&graph);

    let god_list = graphify_analyze::god_nodes(&graph, 5);

    let degrees: Vec<usize> = graph.node_ids().iter().map(|id| graph.degree(id)).collect();
    let avg_degree = if degrees.is_empty() {
        0.0
    } else {
        degrees.iter().sum::<usize>() as f64 / degrees.len() as f64
    };
    let max_degree = degrees.iter().copied().max().unwrap_or(0);

    println!("{}", "Graph Statistics".bold().underline());
    println!("  Nodes:       {}", node_count.to_string().bold());
    println!("  Edges:       {}", edge_count.to_string().bold());
    println!("  Communities: {}", communities.len().to_string().bold());
    println!("  Avg degree:  {avg_degree:.1}");
    println!("  Max degree:  {max_degree}");

    println!("\n{}", "Node Types".bold());
    let mut types: Vec<_> = type_counts.iter().collect();
    types.sort_by(|a, b| b.1.cmp(a.1));
    for (t, count) in &types {
        println!("  {:20} {}", t, count.to_string().cyan());
    }

    println!("\n{}", "Edge Relations".bold());
    let mut rels: Vec<_> = rel_counts.iter().collect();
    rels.sort_by(|a, b| b.1.cmp(a.1));
    for (r, count) in rels.iter().take(15) {
        println!("  {:20} {}", r, count.to_string().cyan());
    }
    if rels.len() > 15 {
        println!("  ... and {} more relation types", rels.len() - 15);
    }

    if !god_list.is_empty() {
        println!("\n{}", "Top Connected Nodes".bold());
        for g in &god_list {
            println!(
                "  {} ({} edges, community {:?})",
                g.label.green(),
                g.degree,
                g.community
            );
        }
    }

    println!("\n  Source: {}", graph_path.dimmed());

    Ok(())
}

/// Initialize a graphify.toml configuration file
fn cmd_init() -> Result<()> {
    let path = Path::new("graphify.toml");
    if path.exists() {
        anyhow::bail!("graphify.toml already exists");
    }
    std::fs::write(
        path,
        r#"# graphify-rs configuration
# These values serve as defaults and can be overridden by CLI flags.

# Output directory for graph files
# output = "graphify-out"

# Disable LLM-based semantic extraction
# no_llm = false

# Only process code files (skip docs/papers)
# code_only = false

# Export formats (comma-separated). Available: json,html,graphml,cypher,svg,wiki,obsidian,report
# Leave empty or omit for all formats.
# formats = ["json", "html", "report"]

# LLM provider for semantic extraction
# [llm]
# provider = "anthropic"          # anthropic | openai | ollama | openai_compatible
# model = "claude-sonnet-4.6"  # required, no default
# anthropic_api_key = "sk-..."    # optional, falls back to ANTHROPIC_API_KEY env or Claude Code OAuth
# anthropic_base_url = "https://api.anthropic.com"  # optional override
# openai_api_key = "sk-..."       # optional, falls back to OPENAI_API_KEY env
# openai_base_url = "https://api.openai.com/v1"     # optional override
# ollama_base_url = "http://localhost:11434"          # optional override
# openai_compatible_api_key = "..."                   # optional
# openai_compatible_base_url = "http://localhost:8000/v1"  # required for openai_compatible
"#,
    )?;
    println!("{} Created graphify.toml", "✓".green());
    Ok(())
}
