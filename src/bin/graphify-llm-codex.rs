use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::Value;

#[derive(Parser, Debug)]
#[command(
    name = "graphify-llm-codex",
    version,
    about = "graphify-rs external LLM adapter backed by an installed Codex CLI"
)]
struct Args {
    /// Codex model to use. Defaults to a currently compatible fast model.
    #[arg(long, default_value = "gpt-5.4-mini")]
    model: String,
    /// Codex reasoning effort.
    #[arg(long, default_value = "low")]
    reasoning_effort: String,
    /// Seconds before killing the Codex subprocess.
    #[arg(long, default_value_t = 120)]
    timeout_secs: u64,
    /// Maximum prompt bytes forwarded to Codex.
    #[arg(long, default_value_t = 120_000)]
    max_prompt_bytes: usize,
    /// Maximum bytes read from Codex final answer file.
    #[arg(long, default_value_t = 64_000)]
    max_output_bytes: usize,
    /// Print the generated Codex prompt instead of invoking Codex.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut source_prompt = String::new();
    io::stdin()
        .read_to_string(&mut source_prompt)
        .context("read graphify prompt from stdin")?;
    if source_prompt.len() > args.max_prompt_bytes {
        source_prompt.truncate(args.max_prompt_bytes);
        source_prompt.push_str("\n\n[graphify prompt truncated by graphify-llm-codex]\n");
    }

    let prompt = codex_prompt(&source_prompt);
    if args.dry_run {
        print!("{prompt}");
        return Ok(());
    }

    let raw = run_codex(&args, &prompt)?;
    let json = extract_semantic_json(&raw)?;
    println!("{}", serde_json::to_string(&json)?);
    Ok(())
}

fn codex_prompt(source_prompt: &str) -> String {
    format!(
        "You are graphify-rs semantic extraction backend.\n\
         Return ONLY compact valid JSON with exactly this shape:\n\
         {{\"entities\":[{{\"name\":\"...\",\"entity_type\":\"concept|class|function|module|paper|image\"}}],\"relationships\":[{{\"source\":\"...\",\"target\":\"...\",\"relation\":\"...\"}}]}}\n\
         Rules:\n\
         - No markdown, no commentary.\n\
         - Prefer 3-20 high-signal entities.\n\
         - Use concise stable names.\n\
         - If existing_extraction is present, update it for current content: preserve valid names, remove stale facts, add new facts.\n\
         - Entity type must be one of: concept, class, function, module, paper, image.\n\
         \nGraphify extraction prompt follows:\n\n{source_prompt}"
    )
}

fn run_codex(args: &Args, prompt: &str) -> Result<String> {
    let prompt_path = temp_path("graphify-codex-prompt");
    let output_path = temp_path("graphify-codex-output");
    std::fs::write(&prompt_path, prompt).context("write temporary Codex prompt")?;

    let prompt_file = std::fs::File::open(&prompt_path).context("open temporary Codex prompt")?;
    let mut child = Command::new("codex")
        .arg("exec")
        .arg("--ephemeral")
        .arg("-m")
        .arg(&args.model)
        .arg("-c")
        .arg(format!(
            "model_reasoning_effort=\"{}\"",
            args.reasoning_effort
        ))
        .arg("-s")
        .arg("read-only")
        .arg("-o")
        .arg(&output_path)
        .arg("-")
        .stdin(Stdio::from(prompt_file))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("start `codex exec`")?;

    let deadline = Instant::now() + Duration::from_secs(args.timeout_secs);
    loop {
        if let Some(status) = child.try_wait().context("poll `codex exec`")? {
            let output = child
                .wait_with_output()
                .context("collect `codex exec` output")?;
            if !status.success() {
                cleanup(&prompt_path, &output_path);
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("codex exec failed with status {status}: {}", stderr.trim());
            }
            let raw = read_limited(&output_path, args.max_output_bytes)
                .context("read Codex final message")?;
            cleanup(&prompt_path, &output_path);
            return Ok(raw);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            cleanup(&prompt_path, &output_path);
            bail!("codex exec timed out after {}s", args.timeout_secs);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn extract_semantic_json(raw: &str) -> Result<Value> {
    let candidate = json_candidate(raw).context("Codex output did not contain a JSON object")?;
    let value: Value = serde_json::from_str(candidate).context("parse semantic JSON from Codex")?;
    validate_semantic_json(&value)?;
    Ok(value)
}

fn validate_semantic_json(value: &Value) -> Result<()> {
    let entities = value
        .get("entities")
        .and_then(Value::as_array)
        .context("missing entities array")?;
    let relationships = value
        .get("relationships")
        .and_then(Value::as_array)
        .context("missing relationships array")?;
    for entity in entities {
        let entity_type = entity
            .get("entity_type")
            .and_then(Value::as_str)
            .context("entity missing entity_type")?;
        if !matches!(
            entity_type,
            "concept" | "class" | "function" | "module" | "paper" | "image"
        ) {
            bail!("unsupported entity_type {entity_type:?}");
        }
        entity
            .get("name")
            .and_then(Value::as_str)
            .context("entity missing name")?;
    }
    for relationship in relationships {
        relationship
            .get("source")
            .and_then(Value::as_str)
            .context("relationship missing source")?;
        relationship
            .get("target")
            .and_then(Value::as_str)
            .context("relationship missing target")?;
        relationship
            .get("relation")
            .and_then(Value::as_str)
            .context("relationship missing relation")?;
    }
    Ok(())
}

fn json_candidate(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    (start < end).then_some(raw[start..=end].trim())
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "{prefix}-{}-{}.tmp",
        std::process::id(),
        monotonic_nanos()
    ));
    path
}

fn monotonic_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

fn read_limited(path: &Path, max_bytes: usize) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(max_bytes as u64)
        .read_to_end(&mut buf)?;
    String::from_utf8(buf).context("Codex output was not UTF-8")
}

fn cleanup(prompt_path: &Path, output_path: &Path) {
    let _ = std::fs::remove_file(prompt_path);
    let _ = std::fs::remove_file(output_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_chatter() {
        let value = extract_semantic_json(
            "codex\n{\"entities\":[{\"name\":\"A\",\"entity_type\":\"concept\"}],\"relationships\":[]}\n",
        )
        .unwrap();
        assert_eq!(value["entities"][0]["name"], "A");
    }

    #[test]
    fn rejects_invalid_entity_type() {
        let err = extract_semantic_json(
            "{\"entities\":[{\"name\":\"A\",\"entity_type\":\"weird\"}],\"relationships\":[]}",
        )
        .unwrap_err();
        assert!(err.to_string().contains("unsupported entity_type"));
    }

    #[test]
    fn dry_prompt_contains_source() {
        let prompt = codex_prompt("hello docs");
        assert!(prompt.contains("hello docs"));
        assert!(prompt.contains("existing_extraction"));
    }
}
