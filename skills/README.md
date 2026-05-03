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
Use `graphifyq` from Codex for short-lived graph access. `graphifyq ensure/query` builds and uses the local Model2Vec semantic index by default; pass `--no-embed` for strict AST-only/offline startup.

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
