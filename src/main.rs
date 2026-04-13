use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

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
enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

impl Verbosity {
    fn from_flags(quiet: bool, verbose: bool) -> Self {
        if quiet {
            Self::Quiet
        } else if verbose {
            Self::Verbose
        } else {
            Self::Normal
        }
    }

    fn is_quiet(self) -> bool {
        matches!(self, Self::Quiet)
    }

    fn is_verbose(self) -> bool {
        matches!(self, Self::Verbose)
    }
}

/// Print helper that respects verbosity.
macro_rules! info_print {
    ($verb:expr, $($arg:tt)*) => {
        if !$verb.is_quiet() {
            println!($($arg)*);
        }
    };
}

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

    // Check skill version staleness on every invocation
    install::check_skill_versions();

    let verb = Verbosity::from_flags(cli.quiet, cli.verbose);

    // Configure tracing based on verbosity
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

    // Configure rayon thread pool if --jobs is set
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
        } => {
            // Merge config file defaults with CLI args
            let cfg = config::load_config(Path::new(&path));
            let effective_path = path;
            let effective_output = if output == "graphify-out" {
                cfg.output.unwrap_or(output)
            } else {
                output
            };
            let effective_no_llm = no_llm || cfg.no_llm.unwrap_or(false);
            let effective_code_only = code_only || cfg.code_only.unwrap_or(false);
            let effective_formats = if format.is_empty() {
                cfg.formats.unwrap_or_default()
            } else {
                format
            };

            cmd_build(
                &effective_path,
                &effective_output,
                effective_no_llm,
                effective_code_only,
                update,
                &effective_formats,
                verb,
                cli.jobs,
            )?;
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
            graphify_serve::start_server(Path::new(&graph)).await?;
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

/// Full build pipeline: detect -> extract (with cache) -> build -> cluster -> analyze -> export
#[allow(clippy::too_many_arguments)]
fn cmd_build(
    path: &str,
    output: &str,
    no_llm: bool,
    code_only: bool,
    update: bool,
    formats: &[String],
    verb: Verbosity,
    jobs: Option<usize>,
) -> Result<()> {
    let root = PathBuf::from(path);
    let output_dir = PathBuf::from(output);
    let cache_dir = output_dir.join("cache");

    // Determine which formats to export (empty = all)
    let all_formats = [
        "json", "html", "graphml", "cypher", "svg", "wiki", "obsidian", "report",
    ];
    let selected: Vec<&str> = if formats.is_empty() {
        all_formats.to_vec()
    } else {
        formats.iter().map(|s| s.as_str()).collect()
    };
    let should_export = |name: &str| selected.iter().any(|s| s.eq_ignore_ascii_case(name));

    // ── Step 1: Detect files ──
    info_print!(verb, "  {} files...", "Detecting".cyan());
    let detection = if update {
        let manifest_path = output_dir.join(".graphify_manifest.json");
        graphify_detect::detect_incremental(&root, Some(manifest_path.to_str().unwrap_or("")))
    } else {
        graphify_detect::detect(&root)
    };
    let n_code = detection
        .files
        .get(&graphify_detect::FileType::Code)
        .map_or(0, |v| v.len());
    let n_doc = detection
        .files
        .get(&graphify_detect::FileType::Document)
        .map_or(0, |v| v.len());
    let n_paper = detection
        .files
        .get(&graphify_detect::FileType::Paper)
        .map_or(0, |v| v.len());
    let n_image = detection
        .files
        .get(&graphify_detect::FileType::Image)
        .map_or(0, |v| v.len());
    info_print!(
        verb,
        "  Found {} files ({} code, {} doc, {} paper, {} image) · ~{} words",
        detection.total_files.to_string().bold(),
        n_code.to_string().green(),
        n_doc.to_string().blue(),
        n_paper.to_string().magenta(),
        n_image.to_string().yellow(),
        detection.total_words
    );
    if let Some(ref warning) = detection.warning {
        info_print!(verb, "  {} {}", "⚠".yellow(), warning.yellow());
    }
    if !detection.skipped_sensitive.is_empty() {
        info_print!(
            verb,
            "  {} Skipped {} sensitive file(s)",
            "⚠".yellow(),
            detection.skipped_sensitive.len()
        );
    }

    // ── Step 2: Extract AST (Pass 1 — deterministic, with per-file cache) ──
    let code_files: Vec<PathBuf> = detection
        .files
        .get(&graphify_detect::FileType::Code)
        .map(|v| v.iter().map(|f| root.join(f)).collect())
        .unwrap_or_default();

    if code_files.is_empty() && code_only {
        info_print!(verb, "  No code files found. Nothing to extract.");
        return Ok(());
    }

    info_print!(
        verb,
        "  {} AST from {} code files...",
        "Extracting".cyan(),
        code_files.len()
    );
    let mut ast_result = graphify_core::model::ExtractionResult::default();
    let mut cache_hits = 0usize;
    let mut extract_errors = 0usize;

    let pb = if !verb.is_quiet() {
        let pb = ProgressBar::new(code_files.len() as u64);
        pb.set_style(
            ProgressStyle::with_template("  {bar:40.cyan/dim} {pos}/{len} files ({eta} remaining)")
                .unwrap()
                .progress_chars("██░"),
        );
        Some(pb)
    } else {
        None
    };

    for file_path in &code_files {
        if let Some(ref pb) = pb {
            pb.set_message(
                file_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );
        }
        // Try loading from cache
        if let Some(cached) = graphify_cache::load_cached_from::<
            graphify_core::model::ExtractionResult,
        >(file_path, &root, &cache_dir)
        {
            cache_hits += 1;
            ast_result.nodes.extend(cached.nodes);
            ast_result.edges.extend(cached.edges);
            ast_result.hyperedges.extend(cached.hyperedges);
            if let Some(ref pb) = pb {
                pb.inc(1);
            }
            continue;
        }
        // Extract fresh — catch panics to not abort the entire pipeline
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            graphify_extract::extract(std::slice::from_ref(file_path))
        })) {
            Ok(fresh) => {
                let _ = graphify_cache::save_cached_to(file_path, &fresh, &root, &cache_dir);
                ast_result.nodes.extend(fresh.nodes);
                ast_result.edges.extend(fresh.edges);
                ast_result.hyperedges.extend(fresh.hyperedges);
            }
            Err(_) => {
                extract_errors += 1;
                verbose_print!(
                    verb,
                    "    {} extraction panicked: {}",
                    "⚠".yellow(),
                    file_path.display()
                );
            }
        }
        if let Some(ref pb) = pb {
            pb.inc(1);
        }
    }
    if let Some(pb) = pb {
        pb.finish_and_clear();
    }
    if cache_hits > 0 {
        info_print!(
            verb,
            "  Cache: {} hits, {} extracted fresh",
            cache_hits.to_string().green(),
            (code_files.len() - cache_hits).to_string().cyan()
        );
    }
    if extract_errors > 0 {
        info_print!(
            verb,
            "  {} {} file(s) had extraction errors (skipped)",
            "⚠".yellow(),
            extract_errors
        );
    }
    info_print!(
        verb,
        "  Pass 1 (AST): {} nodes, {} edges",
        ast_result.nodes.len().to_string().bold(),
        ast_result.edges.len().to_string().bold()
    );

    let mut extractions = vec![ast_result];

    // ── Step 2b: Semantic extraction (Pass 2 — Claude API, concurrent) ──
    if !no_llm && !code_only {
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
        if let Some(key) = api_key {
            let doc_files: Vec<PathBuf> = detection
                .files
                .get(&graphify_detect::FileType::Document)
                .into_iter()
                .chain(detection.files.get(&graphify_detect::FileType::Paper))
                .flat_map(|v| v.iter().map(|f| root.join(f)))
                .collect();

            if !doc_files.is_empty() {
                info_print!(
                    verb,
                    "  {} on {} doc/paper files...",
                    "Semantic extraction".cyan(),
                    doc_files.len()
                );
                let concurrency = jobs.unwrap_or(4).min(8);
                let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
                let rt = tokio::runtime::Handle::current();

                let pb_sem = if !verb.is_quiet() {
                    let pb = ProgressBar::new(doc_files.len() as u64);
                    pb.set_style(
                        ProgressStyle::with_template(
                            "  {bar:40.green/dim} {pos}/{len} docs ({eta} remaining)",
                        )
                        .unwrap()
                        .progress_chars("██░"),
                    );
                    Some(pb)
                } else {
                    None
                };

                // Collect tasks for concurrent execution
                let mut handles = Vec::new();
                for doc_path in &doc_files {
                    // Check cache first (synchronous, on main thread)
                    if let Some(cached) = graphify_cache::load_cached_from::<
                        graphify_core::model::ExtractionResult,
                    >(doc_path, &root, &cache_dir)
                    {
                        extractions.push(cached);
                        if let Some(ref pb) = pb_sem {
                            pb.inc(1);
                        }
                        continue;
                    }
                    let content = match std::fs::read_to_string(doc_path) {
                        Ok(c) => c,
                        Err(_) => {
                            if let Some(ref pb) = pb_sem {
                                pb.inc(1);
                            }
                            continue;
                        }
                    };
                    let file_type = if doc_path.extension().and_then(|e| e.to_str()) == Some("pdf")
                    {
                        "paper"
                    } else {
                        "document"
                    };
                    let doc_p = doc_path.clone();
                    let key_clone = key.clone();
                    let sem_clone = sem.clone();
                    let handle = rt.spawn(async move {
                        let _permit = sem_clone
                            .acquire()
                            .await
                            .map_err(|e| anyhow::anyhow!("semaphore closed: {e}"))?;
                        graphify_extract::semantic::extract_semantic(
                            &doc_p, &content, file_type, &key_clone,
                        )
                        .await
                        .map(|r| (doc_p, r))
                    });
                    handles.push(handle);
                }

                // Collect results
                for handle in handles {
                    match rt.block_on(handle) {
                        Ok(Ok((doc_p, sem_result))) => {
                            verbose_print!(
                                verb,
                                "    {} → {} nodes, {} edges",
                                doc_p.file_name().unwrap_or_default().to_string_lossy(),
                                sem_result.nodes.len(),
                                sem_result.edges.len()
                            );
                            let _ = graphify_cache::save_cached_to(
                                &doc_p,
                                &sem_result,
                                &root,
                                &cache_dir,
                            );
                            extractions.push(sem_result);
                        }
                        Ok(Err(e)) => {
                            verbose_print!(verb, "    {} semantic extraction: {}", "⚠".yellow(), e);
                        }
                        Err(e) => {
                            verbose_print!(verb, "    {} task join error: {}", "⚠".yellow(), e);
                        }
                    }
                    if let Some(ref pb) = pb_sem {
                        pb.inc(1);
                    }
                }
                if let Some(pb) = pb_sem {
                    pb.finish_and_clear();
                }
            }
        } else if n_doc + n_paper > 0 {
            info_print!(
                verb,
                "  {} Set ANTHROPIC_API_KEY to enable semantic extraction for {} doc/paper files",
                "ℹ".blue(),
                n_doc + n_paper
            );
        }
    }

    // ── Step 3: Build graph ──
    info_print!(verb, "  {} graph...", "Building".cyan());
    let mut graph = graphify_build::build(&extractions).context("Failed to build graph")?;
    info_print!(
        verb,
        "  Graph: {} nodes, {} edges",
        graph.node_count().to_string().bold(),
        graph.edge_count().to_string().bold()
    );

    // ── Step 4: Cluster ──
    info_print!(verb, "  {} communities...", "Detecting".cyan());
    let communities = graphify_cluster::cluster(&graph);
    let cohesion = graphify_cluster::score_all(&graph, &communities);

    // Write community assignments back into graph nodes
    for (&cid, members) in &communities {
        for nid in members {
            if let Some(node) = graph.get_node_mut(nid) {
                node.community = Some(cid);
            }
        }
    }

    let community_labels: HashMap<usize, String> = {
        let mut used_labels: std::collections::HashSet<String> = std::collections::HashSet::new();
        communities
            .iter()
            .map(|(cid, nodes)| {
                // Pick the most descriptive label: prefer non-generic names,
                // skip "lib", "super::*", import-like labels, etc.
                let generic = ["lib", "super::*", "main", "mod", "tests"];
                let best = nodes
                    .iter()
                    .filter_map(|id| graph.get_node(id))
                    .filter(|n| {
                        !generic.contains(&n.label.as_str())
                            && !n.label.starts_with("std::")
                            && !n.label.starts_with("serde::")
                            && !n.label.contains("::")
                    })
                    // Prefer functions/structs over file nodes
                    .max_by_key(|n| match n.node_type {
                        graphify_core::model::NodeType::Function => 3,
                        graphify_core::model::NodeType::Class
                        | graphify_core::model::NodeType::Struct => 3,
                        graphify_core::model::NodeType::Module => 1,
                        graphify_core::model::NodeType::File => 0,
                        _ => 2,
                    })
                    .map(|n| n.label.clone())
                    .unwrap_or_else(|| {
                        // Fallback: use first node's label
                        nodes
                            .first()
                            .and_then(|id| graph.get_node(id))
                            .map(|n| n.label.clone())
                            .unwrap_or_else(|| format!("Community {}", cid))
                    });
                // Deduplicate: if label already used, append community id
                let label = if used_labels.contains(&best) {
                    format!("{} ({})", best, cid)
                } else {
                    used_labels.insert(best.clone());
                    best
                };
                (*cid, label)
            })
            .collect()
    };

    info_print!(
        verb,
        "  {} communities detected",
        communities.len().to_string().bold()
    );

    // ── Step 5: Analyze ──
    info_print!(verb, "  {} graph...", "Analyzing".cyan());
    let god_list = graphify_analyze::god_nodes(&graph, 10);
    let surprise_list = graphify_analyze::surprising_connections(&graph, &communities, 5);
    let questions = graphify_analyze::suggest_questions(&graph, &communities, &community_labels, 7);

    // ── Step 6: Export selected formats ──
    std::fs::create_dir_all(&output_dir)?;

    if should_export("json") {
        let json_path = graphify_export::export_json(&graph, &output_dir)?;
        info_print!(verb, "  Wrote {}", json_path.display().to_string().dimmed());
    }

    if should_export("html") {
        let html_path =
            graphify_export::export_html(&graph, &communities, &community_labels, &output_dir)?;
        info_print!(verb, "  Wrote {}", html_path.display().to_string().dimmed());

        // Also generate split HTML (per-community pages)
        let split_path = graphify_export::export_html_split(
            &graph,
            &communities,
            &community_labels,
            &output_dir,
        )?;
        info_print!(
            verb,
            "  Wrote {}/",
            split_path.display().to_string().dimmed()
        );
    }

    // Prepare analysis data
    let detection_json = serde_json::json!({
        "total_files": detection.total_files,
        "total_words": detection.total_words,
        "warning": detection.warning,
    });
    let god_json: Vec<serde_json::Value> = god_list
        .iter()
        .map(
            |g| serde_json::json!({"label": g.label, "degree": g.degree, "community": g.community}),
        )
        .collect();
    let surprise_json: Vec<serde_json::Value> = surprise_list
        .iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();
    let question_json: Vec<serde_json::Value> = questions
        .iter()
        .map(|q| serde_json::to_value(q).unwrap_or_default())
        .collect();
    let token_cost: HashMap<String, usize> =
        HashMap::from([("input".to_string(), 0), ("output".to_string(), 0)]);

    if should_export("report") {
        let report = graphify_export::generate_report(
            &graph,
            &communities,
            &cohesion,
            &community_labels,
            &god_json,
            &surprise_json,
            &detection_json,
            &token_cost,
            path,
            Some(&question_json),
        );
        let report_path = output_dir.join("GRAPH_REPORT.md");
        std::fs::write(&report_path, &report)?;
        info_print!(
            verb,
            "  Wrote {}",
            report_path.display().to_string().dimmed()
        );
    }

    if should_export("graphml") {
        let graphml_path = graphify_export::export_graphml(&graph, &output_dir)?;
        info_print!(
            verb,
            "  Wrote {}",
            graphml_path.display().to_string().dimmed()
        );
    }

    if should_export("cypher") {
        let cypher_path = graphify_export::export_cypher(&graph, &output_dir)?;
        info_print!(
            verb,
            "  Wrote {}",
            cypher_path.display().to_string().dimmed()
        );
    }

    if should_export("svg") {
        let svg_path = graphify_export::export_svg(&graph, &communities, &output_dir)?;
        info_print!(verb, "  Wrote {}", svg_path.display().to_string().dimmed());
    }

    if should_export("wiki") {
        let wiki_path =
            graphify_export::export_wiki(&graph, &communities, &community_labels, &output_dir)?;
        info_print!(verb, "  Wrote {}", wiki_path.display().to_string().dimmed());
    }

    if should_export("obsidian") {
        let obsidian_path =
            graphify_export::export_obsidian(&graph, &communities, &community_labels, &output_dir)?;
        info_print!(
            verb,
            "  Wrote {}",
            obsidian_path.display().to_string().dimmed()
        );
    }

    // Save manifest for future --update runs
    let manifest_path = output_dir.join(".graphify_manifest.json");
    let manifest = graphify_detect::Manifest {
        files: detection
            .files
            .iter()
            .flat_map(|(ft, paths)| paths.iter().map(move |p| (p.clone(), *ft)))
            .collect(),
    };
    graphify_detect::save_manifest(&manifest_path, &manifest)?;

    info_print!(
        verb,
        "\n{} Output in {}",
        "✓ Done!".green().bold(),
        output_dir.display()
    );

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
        .map(|w| w.to_lowercase())
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
    println!("{}", text);

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

        let summary_added = added_nodes.map_or(0, |v| v.len()) + added_edges.map_or(0, |v| v.len());
        let summary_removed =
            removed_nodes.map_or(0, |v| v.len()) + removed_edges.map_or(0, |v| v.len());
        println!(
            "\n{}: {} additions, {} removals",
            "Summary".bold(),
            format!("+{}", summary_added).green(),
            format!("-{}", summary_removed).red()
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

    // Count node types
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(&id) {
            let type_name = format!("{:?}", node.node_type);
            *type_counts.entry(type_name).or_insert(0) += 1;
        }
    }

    // Count edge relations
    let mut rel_counts: HashMap<String, usize> = HashMap::new();
    for edge in graph.edges() {
        *rel_counts.entry(edge.relation.clone()).or_insert(0) += 1;
    }

    // Communities
    let communities = graphify_cluster::cluster(&graph);

    // God nodes
    let god_list = graphify_analyze::god_nodes(&graph, 5);

    // Degree stats
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
    println!("  Avg degree:  {:.1}", avg_degree);
    println!("  Max degree:  {}", max_degree);

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
"#,
    )?;
    println!("{} Created graphify.toml", "✓".green());
    Ok(())
}
