use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 300;

#[derive(Parser)]
#[command(
    name = "graphifyq",
    version,
    about = "Short-lived graphify-rs query helper that manages a local HTTP MCP sidecar"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    /// Build graph if needed and start/reuse the local HTTP sidecar.
    Ensure {
        /// Rebuild .graphify/graph.json even if it already exists.
        #[arg(long)]
        rebuild: bool,
        /// Do not build a missing graph; fail instead.
        #[arg(long)]
        no_build: bool,
        /// Also build .graphify/semantic-index.json with Model2Vec. Enabled by default; kept for compatibility.
        #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_embed")]
        embed: bool,
        /// Disable the default Model2Vec semantic index build.
        #[arg(long = "no-embed", action = ArgAction::SetTrue)]
        no_embed: bool,
        /// Embedding provider: model2vec (local), ollama (local HTTP), or voyage (hosted API).
        #[arg(long, default_value = graphify_embed::DEFAULT_PROVIDER)]
        embedding_provider: String,
        /// Embedding model ID/name for the semantic index.
        #[arg(long, default_value = graphify_embed::DEFAULT_MODEL)]
        embedding_model: String,
        /// Minimum age before graphifyq refreshes an existing graph with `graphify-rs build --update`.
        #[arg(long, default_value_t = DEFAULT_REFRESH_INTERVAL_SECS)]
        refresh_interval_secs: u64,
        /// Disable graphifyq's per-repository auto-refresh check.
        #[arg(long)]
        no_auto_refresh: bool,
        /// Allow graphifyq refresh to run configured external LLM extraction. Default is no new LLM calls.
        #[arg(long)]
        with_llm: bool,
        /// Shell command for external LLM extraction when --with-llm is set.
        #[arg(long)]
        llm_command: Option<String>,
        /// Stable cache label for --llm-command output.
        #[arg(long)]
        llm_provider: Option<String>,
    },
    /// Print sidecar health and registry state.
    Doctor,
    /// Query .graphify/graph.json via the local sidecar.
    Query {
        question: String,
        #[arg(long, default_value_t = 2000)]
        budget: usize,
        /// Ensure a semantic index exists before querying. Enabled by default; kept for compatibility.
        #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_embed")]
        embed: bool,
        /// Disable the default Model2Vec semantic index build before querying.
        #[arg(long = "no-embed", action = ArgAction::SetTrue)]
        no_embed: bool,
        /// Embedding provider: model2vec (local), ollama (local HTTP), or voyage (hosted API).
        #[arg(long, default_value = graphify_embed::DEFAULT_PROVIDER)]
        embedding_provider: String,
        /// Embedding model ID/name for the semantic index.
        #[arg(long, default_value = graphify_embed::DEFAULT_MODEL)]
        embedding_model: String,
        /// Minimum age before graphifyq refreshes an existing graph with `graphify-rs build --update`.
        #[arg(long, default_value_t = DEFAULT_REFRESH_INTERVAL_SECS)]
        refresh_interval_secs: u64,
        /// Disable graphifyq's per-repository auto-refresh check.
        #[arg(long)]
        no_auto_refresh: bool,
        /// Allow graphifyq refresh to run configured external LLM extraction. Default is no new LLM calls.
        #[arg(long)]
        with_llm: bool,
        /// Shell command for external LLM extraction when --with-llm is set.
        #[arg(long)]
        llm_command: Option<String>,
        /// Stable cache label for --llm-command output.
        #[arg(long)]
        llm_provider: Option<String>,
    },
    /// Print graph statistics via the local sidecar.
    Stats,
    /// Generate smart graph summary via MCP.
    Summary {
        #[arg(value_enum, default_value_t = SummaryLevelArg::Community)]
        level: SummaryLevelArg,
        #[arg(long, default_value_t = 2000)]
        budget: usize,
    },
    /// Call a raw MCP tool: graphifyq tool <name> '{"arg":"value"}'
    Tool {
        name: String,
        #[arg(default_value = "{}")]
        arguments: String,
    },
}

#[derive(Clone, Debug, ValueEnum)]
enum SummaryLevelArg {
    Detailed,
    Community,
    Architecture,
}

impl SummaryLevelArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Detailed => "detailed",
            Self::Community => "community",
            Self::Architecture => "architecture",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Registry {
    root: PathBuf,
    pid: u32,
    http_url: String,
    mcp_url: String,
    graphifyq_url: String,
    graph_path: PathBuf,
    started_at_ms: u128,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        CommandKind::Ensure {
            rebuild,
            no_build,
            embed,
            no_embed,
            embedding_provider,
            embedding_model,
            refresh_interval_secs,
            no_auto_refresh,
            with_llm,
            llm_command,
            llm_provider,
        } => {
            let registry = ensure(
                rebuild,
                no_build,
                semantic_enabled(embed, no_embed),
                &embedding_provider,
                &embedding_model,
                RefreshPolicy::new(!no_auto_refresh, refresh_interval_secs),
                build_llm_options(with_llm, llm_command, llm_provider),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&registry)?);
        }
        CommandKind::Doctor => {
            let paths = Paths::discover()?;
            match read_registry(&paths.registry_path) {
                Ok(registry) => {
                    let health = get_json(&format!("{}/health", registry.http_url)).await;
                    println!("registry: {}", paths.registry_path.display());
                    println!("pid: {}", registry.pid);
                    println!("mcp: {}", registry.mcp_url);
                    println!("graph: {}", registry.graph_path.display());
                    match health {
                        Ok(value) => println!("health: {}", serde_json::to_string_pretty(&value)?),
                        Err(err) => println!("health: failed: {err:#}"),
                    }
                }
                Err(err) => {
                    println!("registry: missing or invalid ({err:#})");
                    println!("hint: run `graphifyq ensure`");
                }
            }
        }
        CommandKind::Query {
            question,
            budget,
            embed,
            no_embed,
            embedding_provider,
            embedding_model,
            refresh_interval_secs,
            no_auto_refresh,
            with_llm,
            llm_command,
            llm_provider,
        } => {
            let registry = ensure(
                false,
                false,
                semantic_enabled(embed, no_embed),
                &embedding_provider,
                &embedding_model,
                RefreshPolicy::new(!no_auto_refresh, refresh_interval_secs),
                build_llm_options(with_llm, llm_command, llm_provider),
            )
            .await?;
            let response = post_json(
                &format!("{}/graphifyq/query", registry.http_url),
                &json!({"question": question, "budget": budget}),
            )
            .await?;
            print_tool_response(&response)?;
        }
        CommandKind::Stats => {
            let registry = ensure(
                false,
                false,
                false,
                graphify_embed::DEFAULT_PROVIDER,
                graphify_embed::DEFAULT_MODEL,
                RefreshPolicy::default(),
                None,
            )
            .await?;
            let response = get_json(&format!("{}/graphifyq/stats", registry.http_url)).await?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        CommandKind::Summary { level, budget } => {
            let registry = ensure(
                false,
                false,
                false,
                graphify_embed::DEFAULT_PROVIDER,
                graphify_embed::DEFAULT_MODEL,
                RefreshPolicy::default(),
                None,
            )
            .await?;
            let response = call_tool(
                &registry,
                "smart_summary",
                json!({"level": level.as_str(), "budget": budget}),
            )
            .await?;
            print_tool_response(&response)?;
        }
        CommandKind::Tool { name, arguments } => {
            let require_semantic = name == "semantic_query";
            let registry = ensure(
                false,
                false,
                require_semantic,
                graphify_embed::DEFAULT_PROVIDER,
                graphify_embed::DEFAULT_MODEL,
                RefreshPolicy::default(),
                None,
            )
            .await?;
            let args: Value = serde_json::from_str(&arguments)
                .with_context(|| "tool arguments must be valid JSON")?;
            let response = call_tool(&registry, &name, args).await?;
            print_tool_response(&response)?;
        }
    }

    Ok(())
}

fn semantic_enabled(_embed_flag: bool, no_embed: bool) -> bool {
    // Semantic graph search is the default for graphifyq. `--embed` remains a
    // no-op compatibility flag for older scripts; `--no-embed` is the explicit
    // fast/offline escape hatch.
    !no_embed
}

async fn ensure(
    rebuild: bool,
    no_build: bool,
    embed: bool,
    embedding_provider: &str,
    embedding_model: &str,
    refresh: RefreshPolicy,
    llm: Option<BuildLlmOptions>,
) -> Result<Registry> {
    let paths = Paths::discover()?;
    let outcome = ensure_graph(
        &paths,
        rebuild,
        no_build,
        embed,
        embedding_provider,
        embedding_model,
        refresh,
        llm.as_ref(),
    )?;
    if outcome.ran_build {
        stop_sidecar_if_running(&paths).await;
    }

    if let Ok(registry) = read_registry(&paths.registry_path) {
        if health_ok(&registry, embed).await {
            return Ok(registry);
        }
    }

    start_sidecar(&paths)?;
    wait_for_registry(&paths.registry_path, embed).await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RefreshPolicy {
    enabled: bool,
    interval_secs: u64,
}

impl RefreshPolicy {
    fn new(enabled: bool, interval_secs: u64) -> Self {
        Self {
            enabled,
            interval_secs,
        }
    }
}

impl Default for RefreshPolicy {
    fn default() -> Self {
        Self::new(true, DEFAULT_REFRESH_INTERVAL_SECS)
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct BuildOutcome {
    ran_build: bool,
}

#[derive(Debug, Eq, PartialEq)]
enum BuildReason {
    MissingGraph,
    RebuildRequested,
    MissingSemanticIndex,
    AutoRefresh,
}

#[derive(Debug, Serialize, Deserialize)]
struct RefreshState {
    last_refresh_ms: u128,
}

#[allow(clippy::too_many_arguments)]
fn ensure_graph(
    paths: &Paths,
    rebuild: bool,
    no_build: bool,
    embed: bool,
    embedding_provider: &str,
    embedding_model: &str,
    refresh: RefreshPolicy,
    llm: Option<&BuildLlmOptions>,
) -> Result<BuildOutcome> {
    let embedding_model = normalize_embedding_model(embedding_provider, embedding_model);
    let semantic_path = paths.out_dir.join(graphify_embed::DEFAULT_INDEX_FILE);
    let needs_semantic = embed && !semantic_path.exists();
    let refresh_due = !no_build
        && should_auto_refresh(
            paths.graph_path.exists(),
            &paths.refresh_state_path,
            refresh,
            current_time_ms(),
        );
    let Some(reason) = choose_build_reason(
        paths.graph_path.exists(),
        rebuild,
        needs_semantic,
        refresh_due,
    ) else {
        return Ok(BuildOutcome { ran_build: false });
    };
    if no_build {
        handle_no_build(paths, needs_semantic, &semantic_path)?;
        return Ok(BuildOutcome { ran_build: false });
    }

    let use_update = matches!(reason, BuildReason::AutoRefresh);
    let should_embed = embed;
    run_build(
        paths,
        use_update,
        should_embed,
        embedding_provider,
        &embedding_model,
        llm,
    )?;
    write_refresh_state(&paths.refresh_state_path, current_time_ms())?;
    Ok(BuildOutcome { ran_build: true })
}

fn choose_build_reason(
    graph_exists: bool,
    rebuild: bool,
    needs_semantic: bool,
    refresh_due: bool,
) -> Option<BuildReason> {
    if !graph_exists {
        Some(BuildReason::MissingGraph)
    } else if rebuild {
        Some(BuildReason::RebuildRequested)
    } else if needs_semantic {
        Some(BuildReason::MissingSemanticIndex)
    } else if refresh_due {
        Some(BuildReason::AutoRefresh)
    } else {
        None
    }
}

fn handle_no_build(paths: &Paths, needs_semantic: bool, semantic_path: &Path) -> Result<()> {
    if !paths.graph_path.exists() {
        bail!(
            "graph not found at {}. Run `graphify-rs build --path . --no-llm` first",
            paths.graph_path.display()
        );
    }
    if needs_semantic {
        bail!(
            "semantic index not found at {}. Run `graphify-rs build --path . --no-llm --embed` first",
            semantic_path.display()
        );
    }
    Ok(())
}

fn run_build(
    paths: &Paths,
    update: bool,
    embed: bool,
    embedding_provider: &str,
    embedding_model: &str,
    llm: Option<&BuildLlmOptions>,
) -> Result<()> {
    fs::create_dir_all(paths.graph_path.parent().unwrap_or(&paths.root))?;
    let build_log_path = paths.out_dir.join("graphifyq-build.log");
    let build_log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&build_log_path)
        .with_context(|| format!("open {}", build_log_path.display()))?;
    let build_log_err = build_log.try_clone()?;
    let args = build_command_args(update, embed, embedding_provider, embedding_model, llm);
    let status = Command::new(graphify_rs_exe())
        .current_dir(&paths.root)
        .args(&args)
        .stdout(Stdio::from(build_log))
        .stderr(Stdio::from(build_log_err))
        .status()
        .context("start graphify-rs build")?;
    if !status.success() {
        bail!("graphify-rs build failed with status {status}");
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BuildLlmOptions {
    command: String,
    provider: Option<String>,
}

fn build_llm_options(
    with_llm: bool,
    command: Option<String>,
    provider: Option<String>,
) -> Option<BuildLlmOptions> {
    with_llm.then(|| BuildLlmOptions {
        command: command.unwrap_or_else(|| "graphify-llm-codex".to_string()),
        provider,
    })
}

fn build_command_args(
    update: bool,
    embed: bool,
    embedding_provider: &str,
    embedding_model: &str,
    llm: Option<&BuildLlmOptions>,
) -> Vec<String> {
    let mut args = vec![
        "build".to_string(),
        "--path".to_string(),
        ".".to_string(),
        "--output".to_string(),
        ".graphify".to_string(),
        "--format".to_string(),
        "json,report,context".to_string(),
    ];
    if let Some(llm) = llm {
        args.extend(["--llm-command".to_string(), llm.command.clone()]);
        if let Some(provider) = &llm.provider {
            args.extend(["--llm-provider".to_string(), provider.clone()]);
        }
    } else {
        args.push("--no-llm".to_string());
    }
    if update {
        args.push("--update".to_string());
    }
    if embed {
        args.extend([
            "--embed".to_string(),
            "--embedding-provider".to_string(),
            embedding_provider.to_string(),
            "--embedding-model".to_string(),
            embedding_model.to_string(),
        ]);
    }
    args
}

fn should_auto_refresh(
    graph_exists: bool,
    refresh_state_path: &Path,
    refresh: RefreshPolicy,
    now_ms: u128,
) -> bool {
    if !graph_exists || !refresh.enabled {
        return false;
    }
    let Some(state) = read_refresh_state(refresh_state_path) else {
        return true;
    };
    let interval_ms = u128::from(refresh.interval_secs).saturating_mul(1000);
    now_ms.saturating_sub(state.last_refresh_ms) >= interval_ms
}

fn read_refresh_state(path: &Path) -> Option<RefreshState> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_refresh_state(path: &Path, now_ms: u128) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let state = RefreshState {
        last_refresh_ms: now_ms,
    };
    fs::write(path, serde_json::to_vec_pretty(&state)?)?;
    Ok(())
}

fn current_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn normalize_embedding_model(provider: &str, model: &str) -> String {
    if model != graphify_embed::DEFAULT_MODEL {
        return model.to_string();
    }
    match provider {
        "ollama" => graphify_embed::DEFAULT_OLLAMA_MODEL.to_string(),
        "voyage" | "voyageai" => graphify_embed::DEFAULT_VOYAGE_MODEL.to_string(),
        _ => model.to_string(),
    }
}

fn start_sidecar(paths: &Paths) -> Result<()> {
    fs::create_dir_all(&paths.out_dir)?;
    let log_path = paths.out_dir.join("graphifyq-server.log");
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open {}", log_path.display()))?;
    let log_err = log.try_clone()?;

    let mut command = Command::new(graphify_rs_exe());
    command
        .current_dir(&paths.root)
        .args([
            "serve",
            "--transport",
            "http",
            "--http-bind",
            "127.0.0.1:0",
            "--http-path",
            "/mcp",
            "--registry-path",
        ])
        .arg(&paths.registry_path)
        .arg("--graph")
        .arg(&paths.graph_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    command.spawn().context("start graphify-rs HTTP sidecar")?;

    Ok(())
}

async fn stop_sidecar_if_running(paths: &Paths) {
    let Ok(registry) = read_registry(&paths.registry_path) else {
        return;
    };

    let health = get_json(&format!("{}/health", registry.http_url)).await;
    if health
        .ok()
        .and_then(|value| value["server"].as_str().map(str::to_string))
        .as_deref()
        == Some("graphify-rs")
    {
        let _ = terminate_process(registry.pid);
    }
    let _ = fs::remove_file(&paths.registry_path);
}

fn terminate_process(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        Command::new("kill")
            .arg(pid.to_string())
            .status()
            .context("terminate graphify-rs sidecar")?;
    }
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .context("terminate graphify-rs sidecar")?;
    }
    Ok(())
}

async fn wait_for_registry(path: &Path, require_semantic: bool) -> Result<Registry> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_err: Option<anyhow::Error> = None;
    while Instant::now() < deadline {
        match read_registry(path) {
            Ok(registry) if health_ok(&registry, require_semantic).await => return Ok(registry),
            Ok(_) => {
                last_err = Some(anyhow::anyhow!("sidecar registry exists but health failed"));
            }
            Err(err) => last_err = Some(err),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("sidecar did not start")))
}

async fn health_ok(registry: &Registry, require_semantic: bool) -> bool {
    let Ok(value) = get_json(&format!("{}/health", registry.http_url)).await else {
        return false;
    };
    if !value["ok"].as_bool().unwrap_or(false) {
        return false;
    }
    !require_semantic || !value["semantic"].is_null()
}

async fn call_tool(registry: &Registry, name: &str, arguments: Value) -> Result<Value> {
    post_json(
        &format!("{}/graphifyq/tool", registry.http_url),
        &json!({"name": name, "arguments": arguments}),
    )
    .await
}

async fn get_json(url: &str) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let response = client.get(url).send().await?.error_for_status()?;
    Ok(response.json().await?)
}

async fn post_json(url: &str, body: &Value) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let response = client
        .post(url)
        .json(body)
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

fn read_registry(path: &Path) -> Result<Registry> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read registry {}", path.display()))?;
    Ok(serde_json::from_str(&content)?)
}

fn print_tool_response(response: &Value) -> Result<()> {
    if let Some(content) = response["result"]["content"].as_array() {
        for item in content {
            if let Some(text) = item["text"].as_str() {
                println!("{text}");
            }
        }
        if response["result"]["isError"].as_bool().unwrap_or(false) {
            std::process::exit(2);
        }
    } else {
        println!("{}", serde_json::to_string_pretty(response)?);
    }
    Ok(())
}

fn graphify_rs_exe() -> PathBuf {
    let current = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("graphifyq"));
    let sibling = current.with_file_name(if cfg!(windows) {
        "graphify-rs.exe"
    } else {
        "graphify-rs"
    });
    if sibling.exists() {
        sibling
    } else {
        PathBuf::from("graphify-rs")
    }
}

struct Paths {
    root: PathBuf,
    out_dir: PathBuf,
    graph_path: PathBuf,
    registry_path: PathBuf,
    refresh_state_path: PathBuf,
}

impl Paths {
    fn discover() -> Result<Self> {
        let root = std::env::current_dir()
            .context("current directory")?
            .canonicalize()
            .context("canonicalize current directory")?;
        let out_dir = root.join(".graphify");
        Ok(Self {
            graph_path: out_dir.join("graph.json"),
            registry_path: out_dir.join(".graphifyq-server.json"),
            refresh_state_path: out_dir.join(".graphifyq-refresh.json"),
            out_dir,
            root,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_level_arg_serializes_to_mcp_values() {
        assert_eq!(SummaryLevelArg::Detailed.as_str(), "detailed");
        assert_eq!(SummaryLevelArg::Community.as_str(), "community");
        assert_eq!(SummaryLevelArg::Architecture.as_str(), "architecture");
    }

    #[test]
    fn semantic_mode_defaults_on_and_allows_opt_out() {
        assert!(semantic_enabled(false, false));
        assert!(semantic_enabled(true, false));
        assert!(!semantic_enabled(false, true));
    }

    #[test]
    fn registry_deserializes_from_sidecar_json() {
        let registry: Registry = serde_json::from_value(json!({
            "root": "/tmp/project",
            "pid": 1234,
            "http_url": "http://127.0.0.1:12345",
            "mcp_url": "http://127.0.0.1:12345/mcp",
            "graphifyq_url": "http://127.0.0.1:12345/graphifyq",
            "graph_path": "/tmp/project/.graphify/graph.json",
            "started_at_ms": 42
        }))
        .unwrap();

        assert_eq!(registry.pid, 1234);
        assert_eq!(registry.mcp_url, "http://127.0.0.1:12345/mcp");
        assert_eq!(
            registry.graph_path,
            PathBuf::from("/tmp/project/.graphify/graph.json")
        );
    }

    #[test]
    fn build_command_args_use_update_only_for_refresh_path() {
        let args = build_command_args(true, true, "ollama", "embeddinggemma", None);

        assert!(args.iter().any(|arg| arg == "--update"));
        assert!(args.iter().any(|arg| arg == "--embed"));
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--embedding-provider" && pair[1] == "ollama")
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--embedding-model" && pair[1] == "embeddinggemma")
        );

        let args = build_command_args(false, false, "model2vec", "minishlab/potion-base-8M", None);
        assert!(!args.iter().any(|arg| arg == "--update"));
        assert!(!args.iter().any(|arg| arg == "--embed"));
    }

    #[test]
    fn build_command_args_supports_explicit_llm_refresh() {
        let llm = BuildLlmOptions {
            command: "graphify-llm-codex --model gpt-5.4-mini".to_string(),
            provider: Some("codex".to_string()),
        };
        let args = build_command_args(
            false,
            true,
            "model2vec",
            "minishlab/potion-code-16M",
            Some(&llm),
        );
        assert!(!args.iter().any(|arg| arg == "--no-llm"));
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--llm-command" && pair[1] == llm.command)
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--llm-provider" && pair[1] == "codex")
        );
    }

    #[test]
    fn auto_refresh_is_due_on_missing_or_expired_state() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("refresh.json");
        let refresh = RefreshPolicy::new(true, 300);

        assert!(should_auto_refresh(true, &state_path, refresh, 1_000));

        write_refresh_state(&state_path, 1_000).unwrap();
        assert!(!should_auto_refresh(true, &state_path, refresh, 299_999));
        assert!(should_auto_refresh(true, &state_path, refresh, 301_000));
        assert!(!should_auto_refresh(
            true,
            &state_path,
            RefreshPolicy::new(false, 300),
            301_000
        ));
        assert!(!should_auto_refresh(false, &state_path, refresh, 301_000));
    }

    #[test]
    fn build_reason_prioritizes_correctness_before_refresh() {
        assert_eq!(
            choose_build_reason(false, false, false, true),
            Some(BuildReason::MissingGraph)
        );
        assert_eq!(
            choose_build_reason(true, true, true, true),
            Some(BuildReason::RebuildRequested)
        );
        assert_eq!(
            choose_build_reason(true, false, true, true),
            Some(BuildReason::MissingSemanticIndex)
        );
        assert_eq!(
            choose_build_reason(true, false, false, true),
            Some(BuildReason::AutoRefresh)
        );
        assert_eq!(choose_build_reason(true, false, false, false), None);
    }
}
