# LLM Provider Support for Semantic Extraction

## Problem

The `--semantic` extraction currently only supports Anthropic Claude via `ANTHROPIC_API_KEY`. Users want to use other LLM providers (OpenAI, Ollama, local models) and Claude Code's OAuth token.

## Design Decisions

- **Providers**: Anthropic Claude (with OAuth), OpenAI, Ollama, generic OpenAI-compatible endpoints
- **Anthropic OAuth**: Reuse Claude Code's locally stored OAuth token (`~/.claude/credentials.json`)
- **Configuration**: `graphify.toml` only (no CLI flags for provider)
- **Base URLs**: Built-in defaults per provider, overridable in config
- **Model name**: No default — must be explicitly specified
- **Architecture**: Dual-path — Anthropic (own API format) + OpenAI-compatible (shared implementation)

## 1. Configuration

New `[llm]` section in `graphify.toml`:

```toml
[llm]
provider = "anthropic"       # anthropic | openai | ollama | openai_compatible
model = "claude-sonnet-4.6"  # required, no default

# Anthropic (optional overrides)
# anthropic_api_key = "sk-..."
# anthropic_base_url = "https://api.anthropic.com"

# OpenAI (optional overrides)
# openai_api_key = "sk-..."
# openai_base_url = "https://api.openai.com/v1"

# Ollama (optional overrides)
# ollama_base_url = "http://localhost:11434"

# openai_compatible
# openai_compatible_api_key = "..."
# openai_compatible_base_url = "http://localhost:8000/v1"  # required for this provider
```

**API key resolution priority**: config field > environment variable (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY`) > Claude Code OAuth token (Anthropic only)

**Backward compatibility**: If `[llm]` section is absent, fall back to current behavior (check `ANTHROPIC_API_KEY`, use Anthropic if present, skip semantic extraction otherwise).

## 2. Provider Architecture

New `provider.rs` in `graphify-extract/src/semantic/`:

```rust
pub enum LLMProvider {
    Anthropic,
    OpenAI,
    Ollama,
    OpenAICompatible,
}

pub struct LLMConfig {
    pub provider: LLMProvider,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: String,
}
```

Dispatch logic:

- `LLMProvider::Anthropic` → `extract_anthropic()`: Anthropic Messages API format, OAuth support
- `LLMProvider::OpenAI | Ollama | OpenAICompatible` → `extract_openai_compatible()`: OpenAI Chat Completions API format

`extract_semantic()` signature changes from `(path, content, file_type, api_key)` to `(path, content, file_type, &LLMConfig)`.

Shared: prompt construction (`build_system_prompt`, `build_user_prompt`) and response parsing (`parse_semantic_response`) are reused by both paths.

## 3. Anthropic Path + OAuth Token Reuse

New `anthropic_oauth.rs` module:

```rust
/// Read OAuth access token from Claude Code's local storage
/// Path: ~/.claude/credentials.json
/// Returns None if file missing or format mismatch
pub fn read_claude_code_oauth_token() -> Option<String>
```

API key resolution chain (resolved at `LLMConfig` construction time):

1. `anthropic_api_key` field in `graphify.toml`
2. `ANTHROPIC_API_KEY` environment variable
3. `read_claude_code_oauth_token()`

When using OAuth token, HTTP header changes from `x-api-key` to `Authorization: Bearer <token>`. Rest of Anthropic Messages API format unchanged.

Token expiration: graphify-rs does not refresh tokens. On 401, error message directs user to run `claude login`.

## 4. OpenAI-Compatible Path

Shared implementation for OpenAI, Ollama, and OpenAICompatible providers using Chat Completions API:

```
POST {base_url}/chat/completions
Authorization: Bearer {api_key}
Body: { "model": "...", "max_tokens": 4096, "messages": [...], "system": "..." }
```

Provider defaults:

| Provider | Default base_url | API key |
|---|---|---|
| OpenAI | `https://api.openai.com/v1` | Required (config > `OPENAI_API_KEY` env) |
| Ollama | `http://localhost:11434/v1` | Not needed |
| OpenAICompatible | Must be user-specified | Optional |

Response parsing: extract `choices[0].message.content`, then feed to existing `parse_semantic_response()`.

## 5. CLI Integration

`main.rs` `cmd_build` semantic extraction section changes:

1. Read `graphify.toml` → parse `[llm]` → build `LLMConfig`
2. If `[llm]` absent: fall back to current behavior (check `ANTHROPIC_API_KEY`)
3. Dispatch to provider-specific path based on `LLMConfig.provider`

`config.rs` gains `llm: Option<LLMConfig>` field on `Config`.

`cmd_init` template updated with commented `[llm]` example.

`--no_llm` flag still takes priority over `[llm]` config.

## 6. Error Handling

**Auth failures**:
- Anthropic 401 → "Anthropic API key invalid or OAuth token expired. Run `claude login` to refresh, or set ANTHROPIC_API_KEY."
- OpenAI 401 → "OpenAI API key invalid. Set OPENAI_API_KEY or configure in graphify.toml."
- Ollama/OpenAICompatible connection failure → "Cannot connect to {base_url}. Make sure the server is running."

**Model not found**:
- Anthropic 400/404 → "Model '{model}' not found. Check available models at docs.anthropic.com"
- OpenAI 404 → "Model '{model}' not found. Check available models with: openai api models.list"
- Ollama 404 → "Model '{model}' not found. Run: ollama pull {model}"

**Non-blocking**: Semantic extraction failures print a warning and continue the build pipeline (current behavior preserved).
