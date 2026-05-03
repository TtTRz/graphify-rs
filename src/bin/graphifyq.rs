use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    },
    /// Print sidecar health and registry state.
    Doctor,
    /// Query .graphify/graph.json via the local sidecar.
    Query {
        question: String,
        #[arg(long, default_value_t = 2000)]
        budget: usize,
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
        CommandKind::Ensure { rebuild, no_build } => {
            let registry = ensure(rebuild, no_build).await?;
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
        CommandKind::Query { question, budget } => {
            let registry = ensure(false, false).await?;
            let response = post_json(
                &format!("{}/graphifyq/query", registry.http_url),
                &json!({"question": question, "budget": budget}),
            )
            .await?;
            print_tool_response(&response)?;
        }
        CommandKind::Stats => {
            let registry = ensure(false, false).await?;
            let response = get_json(&format!("{}/graphifyq/stats", registry.http_url)).await?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        CommandKind::Summary { level, budget } => {
            let registry = ensure(false, false).await?;
            let response = call_tool(
                &registry,
                "smart_summary",
                json!({"level": level.as_str(), "budget": budget}),
            )
            .await?;
            print_tool_response(&response)?;
        }
        CommandKind::Tool { name, arguments } => {
            let registry = ensure(false, false).await?;
            let args: Value = serde_json::from_str(&arguments)
                .with_context(|| "tool arguments must be valid JSON")?;
            let response = call_tool(&registry, &name, args).await?;
            print_tool_response(&response)?;
        }
    }

    Ok(())
}

async fn ensure(rebuild: bool, no_build: bool) -> Result<Registry> {
    let paths = Paths::discover()?;
    ensure_graph(&paths, rebuild, no_build)?;

    if let Ok(registry) = read_registry(&paths.registry_path) {
        if health_ok(&registry).await {
            return Ok(registry);
        }
    }

    start_sidecar(&paths)?;
    wait_for_registry(&paths.registry_path).await
}

fn ensure_graph(paths: &Paths, rebuild: bool, no_build: bool) -> Result<()> {
    if paths.graph_path.exists() && !rebuild {
        return Ok(());
    }
    if no_build {
        bail!(
            "graph not found at {}. Run `graphify-rs build --path . --no-llm` first",
            paths.graph_path.display()
        );
    }

    fs::create_dir_all(paths.graph_path.parent().unwrap_or(&paths.root))?;
    let build_log_path = paths.out_dir.join("graphifyq-build.log");
    let build_log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&build_log_path)
        .with_context(|| format!("open {}", build_log_path.display()))?;
    let build_log_err = build_log.try_clone()?;
    let status = Command::new(graphify_rs_exe())
        .current_dir(&paths.root)
        .args([
            "build",
            "--path",
            ".",
            "--output",
            ".graphify",
            "--no-llm",
            "--format",
            "json,report",
        ])
        .stdout(Stdio::from(build_log))
        .stderr(Stdio::from(build_log_err))
        .status()
        .context("start graphify-rs build")?;
    if !status.success() {
        bail!("graphify-rs build failed with status {status}");
    }
    Ok(())
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

async fn wait_for_registry(path: &Path) -> Result<Registry> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_err: Option<anyhow::Error> = None;
    while Instant::now() < deadline {
        match read_registry(path) {
            Ok(registry) if health_ok(&registry).await => return Ok(registry),
            Ok(_) => {
                last_err = Some(anyhow::anyhow!("sidecar registry exists but health failed"));
            }
            Err(err) => last_err = Some(err),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("sidecar did not start")))
}

async fn health_ok(registry: &Registry) -> bool {
    get_json(&format!("{}/health", registry.http_url))
        .await
        .ok()
        .and_then(|v| v["ok"].as_bool())
        .unwrap_or(false)
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
}
