use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::skill::SKILL_CONTENT;

/// Current package version for staleness checks.
const VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Platform configs ──

struct PlatformConfig {
    skill_dst: &'static str,
    register_claude_md: bool,
}

const PLATFORMS: &[(&str, PlatformConfig)] = &[
    (
        "claude",
        PlatformConfig {
            skill_dst: ".claude/skills/graphify/SKILL.md",
            register_claude_md: true,
        },
    ),
    (
        "codex",
        PlatformConfig {
            skill_dst: ".agents/skills/graphify/SKILL.md",
            register_claude_md: false,
        },
    ),
    (
        "opencode",
        PlatformConfig {
            skill_dst: ".config/opencode/skills/graphify/SKILL.md",
            register_claude_md: false,
        },
    ),
    (
        "claw",
        PlatformConfig {
            skill_dst: ".claw/skills/graphify/SKILL.md",
            register_claude_md: false,
        },
    ),
    (
        "droid",
        PlatformConfig {
            skill_dst: ".factory/skills/graphify/SKILL.md",
            register_claude_md: false,
        },
    ),
    (
        "trae",
        PlatformConfig {
            skill_dst: ".trae/skills/graphify/SKILL.md",
            register_claude_md: false,
        },
    ),
    (
        "trae-cn",
        PlatformConfig {
            skill_dst: ".trae-cn/skills/graphify/SKILL.md",
            register_claude_md: false,
        },
    ),
    (
        "windows",
        PlatformConfig {
            skill_dst: ".claude/skills/graphify/SKILL.md",
            register_claude_md: true,
        },
    ),
];

// ── Registration text for ~/.claude/CLAUDE.md ──

const SKILL_REGISTRATION: &str = r#"
# graphify
- **graphify** (`~/.claude/skills/graphify/SKILL.md`) - any input to knowledge graph. Trigger: `/graphify`
When the user types `/graphify`, invoke the Skill tool with `skill: "graphify"` before doing anything else.
"#;

// ── Project-level CLAUDE.md section ──

const CLAUDE_MD_SECTION: &str = r#"## graphify

This project has a graphify-rs knowledge graph at graphify-out/.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- After modifying code files in this session, run `graphify-rs build .` to keep the graph current
"#;

const CLAUDE_MD_MARKER: &str = "## graphify";

// ── AGENTS.md section (for non-claude platforms) ──

const AGENTS_MD_SECTION: &str = r#"## graphify

This project has a graphify-rs knowledge graph at graphify-out/.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- After modifying code files in this session, run `graphify-rs build .` to keep the graph current
"#;

const AGENTS_MD_MARKER: &str = "## graphify";

// ── Public API ──

/// Check all known skill install locations for stale versions.
/// Call this on startup (before executing any subcommand).
pub fn check_skill_versions() {
    let home = match home_dir() {
        Ok(h) => h,
        Err(_) => return,
    };
    for (_, config) in PLATFORMS {
        let version_file = home
            .join(config.skill_dst)
            .parent()
            .map(|p| p.join(".graphify_version"))
            .unwrap_or_default();
        if version_file.exists() {
            if let Ok(installed) = fs::read_to_string(&version_file) {
                let installed = installed.trim();
                if !installed.is_empty() && installed != VERSION {
                    eprintln!(
                        "  warning: skill is from graphify-rs {}, package is {}. Run 'graphify-rs install' to update.",
                        installed, VERSION
                    );
                    return; // Only warn once
                }
            }
        }
    }
}

/// Install graphify skill file for a given platform (global install).
pub fn install_skill(platform: &str) -> Result<()> {
    let config = PLATFORMS
        .iter()
        .find(|(name, _)| *name == platform)
        .map(|(_, cfg)| cfg)
        .with_context(|| {
            let valid: Vec<&str> = PLATFORMS.iter().map(|(n, _)| *n).collect();
            format!(
                "Unknown platform '{}'. Valid platforms: {}",
                platform,
                valid.join(", ")
            )
        })?;

    let home = home_dir()?;
    let skill_path = home.join(config.skill_dst);

    // Create parent directories
    if let Some(parent) = skill_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    // Write skill file
    fs::write(&skill_path, SKILL_CONTENT)
        .with_context(|| format!("Failed to write skill file to {}", skill_path.display()))?;
    println!("  Wrote skill file to {}", skill_path.display());

    // Write version stamp for staleness detection
    if let Some(parent) = skill_path.parent() {
        let version_file = parent.join(".graphify_version");
        let _ = fs::write(&version_file, VERSION);
    }

    // Register in CLAUDE.md if needed
    if config.register_claude_md {
        let claude_md_path = home.join(".claude/CLAUDE.md");
        register_in_file(&claude_md_path, SKILL_REGISTRATION, "# graphify")?;
        println!("  Registered in {}", claude_md_path.display());
    }

    println!("\n  Installed graphify skill for '{}'.", platform);
    println!("  Use `/graphify` in your AI assistant to trigger the skill.");

    Ok(())
}

/// `graphify-rs claude install` — project-level Claude integration.
pub fn claude_install(project_root: &Path) -> Result<()> {
    // 1. Append section to ./CLAUDE.md
    let claude_md = project_root.join("CLAUDE.md");
    append_section(&claude_md, CLAUDE_MD_SECTION, CLAUDE_MD_MARKER)?;
    println!("  Updated {}", claude_md.display());

    // 2. Write PreToolUse hook to .claude/settings.json
    let settings_path = project_root.join(".claude/settings.json");
    write_claude_settings_hook(&settings_path)?;
    println!("  Wrote hook to {}", settings_path.display());

    println!("\n  Claude integration installed.");
    Ok(())
}

/// `graphify-rs claude uninstall` — remove project-level Claude integration.
pub fn claude_uninstall(project_root: &Path) -> Result<()> {
    // 1. Remove section from ./CLAUDE.md
    let claude_md = project_root.join("CLAUDE.md");
    remove_section(&claude_md, CLAUDE_MD_MARKER)?;
    println!("  Cleaned {}", claude_md.display());

    // 2. Remove hook from .claude/settings.json
    let settings_path = project_root.join(".claude/settings.json");
    remove_claude_settings_hook(&settings_path)?;
    println!("  Cleaned {}", settings_path.display());

    println!("\n  Claude integration uninstalled.");
    Ok(())
}

/// `graphify-rs codex install` — project-level Codex integration.
pub fn codex_install(project_root: &Path) -> Result<()> {
    let agents_md = project_root.join("AGENTS.md");
    append_section(&agents_md, AGENTS_MD_SECTION, AGENTS_MD_MARKER)?;
    println!("  Updated {}", agents_md.display());

    // Write hook to .codex/hooks.json
    let hooks_path = project_root.join(".codex/hooks.json");
    write_codex_hooks(&hooks_path)?;
    println!("  Wrote hook to {}", hooks_path.display());

    println!("\n  Codex integration installed.");
    Ok(())
}

/// `graphify-rs codex uninstall`
pub fn codex_uninstall(project_root: &Path) -> Result<()> {
    let agents_md = project_root.join("AGENTS.md");
    remove_section(&agents_md, AGENTS_MD_MARKER)?;
    println!("  Cleaned {}", agents_md.display());

    let hooks_path = project_root.join(".codex/hooks.json");
    if hooks_path.exists() {
        fs::remove_file(&hooks_path)?;
        println!("  Removed {}", hooks_path.display());
    }

    println!("\n  Codex integration uninstalled.");
    Ok(())
}

/// `graphify-rs opencode install` — project-level OpenCode integration.
pub fn opencode_install(project_root: &Path) -> Result<()> {
    let agents_md = project_root.join("AGENTS.md");
    append_section(&agents_md, AGENTS_MD_SECTION, AGENTS_MD_MARKER)?;
    println!("  Updated {}", agents_md.display());

    // Write plugin
    let plugin_path = project_root.join(".opencode/plugins/graphify.js");
    write_opencode_plugin(&plugin_path)?;
    println!("  Wrote plugin to {}", plugin_path.display());

    // Register in opencode.json
    let config_path = project_root.join("opencode.json");
    register_opencode_config(&config_path)?;
    println!("  Updated {}", config_path.display());

    println!("\n  OpenCode integration installed.");
    Ok(())
}

/// `graphify-rs opencode uninstall`
pub fn opencode_uninstall(project_root: &Path) -> Result<()> {
    let agents_md = project_root.join("AGENTS.md");
    remove_section(&agents_md, AGENTS_MD_MARKER)?;
    println!("  Cleaned {}", agents_md.display());

    let plugin_path = project_root.join(".opencode/plugins/graphify.js");
    if plugin_path.exists() {
        fs::remove_file(&plugin_path)?;
        println!("  Removed {}", plugin_path.display());
    }

    // Remove from opencode.json
    let config_path = project_root.join("opencode.json");
    unregister_opencode_config(&config_path)?;
    println!("  Cleaned {}", config_path.display());

    println!("\n  OpenCode integration uninstalled.");
    Ok(())
}

/// Generic platform install — just writes AGENTS.md section.
pub fn generic_platform_install(project_root: &Path, platform: &str) -> Result<()> {
    let agents_md = project_root.join("AGENTS.md");
    append_section(&agents_md, AGENTS_MD_SECTION, AGENTS_MD_MARKER)?;
    println!("  Updated {}", agents_md.display());
    println!("\n  {} integration installed.", platform);
    Ok(())
}

/// Generic platform uninstall — just removes AGENTS.md section.
pub fn generic_platform_uninstall(project_root: &Path, platform: &str) -> Result<()> {
    let agents_md = project_root.join("AGENTS.md");
    remove_section(&agents_md, AGENTS_MD_MARKER)?;
    println!("  Cleaned {}", agents_md.display());
    println!("\n  {} integration uninstalled.", platform);
    Ok(())
}

// ── Helpers ──

fn home_dir() -> Result<std::path::PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

/// Append a section to a file if the marker is not already present.
fn append_section(path: &Path, section: &str, marker: &str) -> Result<()> {
    let existing = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    if existing.contains(marker) {
        println!("  Section already present in {}, skipping.", path.display());
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push('\n');
    content.push_str(section);

    fs::write(path, content)?;
    Ok(())
}

/// Remove a section from a file identified by a marker line.
/// Removes from the marker line until the next `##` heading or end of file.
fn remove_section(path: &Path, marker: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path)?;
    if !content.contains(marker) {
        return Ok(());
    }

    let mut result = String::new();
    let mut skipping = false;

    for line in content.lines() {
        if line.starts_with(marker) {
            skipping = true;
            continue;
        }
        if skipping {
            // Stop skipping at next ## heading or end
            if line.starts_with("## ") {
                skipping = false;
                result.push_str(line);
                result.push('\n');
            }
            // else: skip this line
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }

    // Trim trailing whitespace
    let trimmed = result.trim_end().to_string() + "\n";
    fs::write(path, trimmed)?;
    Ok(())
}

/// Register a skill reference in a file (like ~/.claude/CLAUDE.md).
fn register_in_file(path: &Path, registration_text: &str, marker: &str) -> Result<()> {
    let existing = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    if existing.contains(marker) {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(registration_text);

    fs::write(path, content)?;
    Ok(())
}

/// Write Claude PreToolUse hook to .claude/settings.json.
fn write_claude_settings_hook(path: &Path) -> Result<()> {
    let mut settings: serde_json::Value = if path.exists() {
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let hook_entry = serde_json::json!({
        "matcher": "Glob|Grep",
        "hooks": [{
            "type": "command",
            "command": "[ -f graphify-out/graph.json ] && echo '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"additionalContext\":\"graphify-rs: Knowledge graph exists. Read graphify-out/GRAPH_REPORT.md for god nodes and community structure before searching raw files.\"}}' || true"
        }]
    });

    // Ensure hooks.PreToolUse exists as an array
    let hooks = settings
        .as_object_mut()
        .context("settings is not an object")?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let pre_tool_use = hooks
        .as_object_mut()
        .context("hooks is not an object")?
        .entry("PreToolUse")
        .or_insert_with(|| serde_json::json!([]));

    let arr = pre_tool_use
        .as_array_mut()
        .context("PreToolUse is not an array")?;

    // Check if already present (by matcher)
    let already = arr
        .iter()
        .any(|v| v.get("matcher").and_then(|m| m.as_str()) == Some("Glob|Grep"));
    if !already {
        arr.push(hook_entry);
    }

    let output = serde_json::to_string_pretty(&settings)?;
    fs::write(path, output)?;
    Ok(())
}

/// Remove Claude PreToolUse hook from .claude/settings.json.
fn remove_claude_settings_hook(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path)?;
    let mut settings: serde_json::Value =
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));

    if let Some(hooks) = settings.get_mut("hooks") {
        if let Some(pre_tool_use) = hooks.get_mut("PreToolUse") {
            if let Some(arr) = pre_tool_use.as_array_mut() {
                arr.retain(|v| v.get("matcher").and_then(|m| m.as_str()) != Some("Glob|Grep"));
            }
        }
    }

    let output = serde_json::to_string_pretty(&settings)?;
    fs::write(path, output)?;
    Ok(())
}

/// Write Codex hooks.json.
fn write_codex_hooks(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let hooks = serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "Bash",
                "hooks": [{
                    "type": "command",
                    "command": "[ -f graphify-out/graph.json ] && echo '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"allow\",\"systemMessage\":\"graphify-rs: Knowledge graph exists. Read graphify-out/GRAPH_REPORT.md for god nodes and community structure before searching raw files.\"}}' || true"
                }]
            }]
        }
    });

    let output = serde_json::to_string_pretty(&hooks)?;
    fs::write(path, output)?;
    Ok(())
}

/// Write OpenCode plugin file.
fn write_opencode_plugin(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let plugin_content = r#"// graphify-rs plugin for OpenCode
module.exports = {
    name: "graphify-rs",
    description: "Knowledge graph integration",
    hooks: {
        preToolUse: async (ctx) => {
            const fs = require("fs");
            if (fs.existsSync("graphify-out/graph.json")) {
                return {
                    prefix: "[graphify-rs] Knowledge graph available. Read graphify-out/GRAPH_REPORT.md for architecture overview."
                };
            }
        }
    }
};
"#;

    fs::write(path, plugin_content)?;
    Ok(())
}

/// Register graphify plugin in opencode.json.
fn register_opencode_config(path: &Path) -> Result<()> {
    let mut config: serde_json::Value = if path.exists() {
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let plugins = config
        .as_object_mut()
        .context("config is not an object")?
        .entry("plugins")
        .or_insert_with(|| serde_json::json!([]));

    if let Some(arr) = plugins.as_array_mut() {
        let already = arr
            .iter()
            .any(|v| v.as_str() == Some(".opencode/plugins/graphify.js"));
        if !already {
            arr.push(serde_json::json!(".opencode/plugins/graphify.js"));
        }
    }

    let output = serde_json::to_string_pretty(&config)?;
    fs::write(path, output)?;
    Ok(())
}

/// Remove graphify plugin from opencode.json.
fn unregister_opencode_config(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path)?;
    let mut config: serde_json::Value =
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));

    if let Some(plugins) = config.get_mut("plugins") {
        if let Some(arr) = plugins.as_array_mut() {
            arr.retain(|v| v.as_str() != Some(".opencode/plugins/graphify.js"));
        }
    }

    let output = serde_json::to_string_pretty(&config)?;
    fs::write(path, output)?;
    Ok(())
}
