# graphify-rs agent skills

This directory contains ready-to-copy skills for assistants that support local
skill folders.

## Codex

Recommended global install path:

```bash
mkdir -p ~/.codex/skills/graphify
cp skills/codex/graphify/SKILL.md ~/.codex/skills/graphify/SKILL.md
```

Or install through the binary:

```bash
graphify-rs install --platform codex
```

This writes the skill to `~/.codex/skills/graphify/SKILL.md`.

For a project-level Codex setup, run from a repository root:

```bash
graphify-rs codex install
```

This writes `AGENTS.md` guidance and a `.codex/hooks.json` hook-check entry.
Use `graphifyq` from Codex for short-lived graph access. `graphifyq ensure/query` builds and uses the local Model2Vec semantic index by default, auto-refreshes stale per-repo graphs every 300s, and restarts its local HTTP sidecar after refresh; pass `--no-embed` for strict AST-only/offline startup or `--no-auto-refresh` for read-only checks.

LLM enrichment is explicit and can use installed CLIs instead of API keys. For
Codex-backed extraction:

```bash
graphifyq ensure --with-llm \
  --llm-command "graphify-llm-codex --model gpt-5.4-mini --reasoning-effort low" \
  --llm-provider codex-cli
```

Regular `graphifyq ensure` / `graphify-rs build --no-llm` makes no new LLM calls
and preserves existing cached LLM output.

## Claude Code

Recommended global install path:

```bash
mkdir -p ~/.claude/skills/graphify
cp skills/claude/graphify/SKILL.md ~/.claude/skills/graphify/SKILL.md
```

Or install through the binary:

```bash
graphify-rs install --platform claude
```

For a project-level Claude Code setup, run from a repository root:

```bash
graphify-rs claude install
```

This writes `CLAUDE.md` guidance and a `.claude/settings.json` PreToolUse hook.
