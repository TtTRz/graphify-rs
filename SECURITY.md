# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.4.x   | ✅ Active |
| < 0.4   | ❌ Not supported |

## Reporting a Vulnerability

If you discover a security vulnerability in graphify-rs, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead:

1. **Email**: Send a detailed report to the maintainer via GitHub private vulnerability reporting
2. **GitHub Security Advisory**: Use [GitHub's security advisory feature](https://github.com/TtTRz/graphify-rs/security/advisories/new) to submit a private report

### What to include

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

### Response timeline

- **Acknowledgment**: Within 48 hours
- **Initial assessment**: Within 1 week
- **Fix release**: Within 2 weeks for critical issues

## Security Architecture

graphify-rs includes a dedicated `graphify-security` crate that provides:

- **URL validation** — SSRF prevention for URL ingestion (blocks private IPs, localhost)
- **Path traversal protection** — sanitizes file paths in exports (Obsidian, Wiki)
- **Label injection defense** — escapes HTML/JS in node labels for visualization
- **Filename length safety** — `truncate_to_bytes()` prevents OS filename limit crashes

## Dependency Auditing

We regularly run `cargo audit` to check for known vulnerabilities in dependencies. You can run it yourself:

```bash
cargo audit
```

## Scope

The following are **in scope** for security reports:

- Remote code execution
- Path traversal in file exports
- SSRF via URL ingestion
- Denial of service (e.g., stack overflow on malicious input)
- Information disclosure via MCP server

The following are **out of scope**:

- Local file access (graphify-rs reads files by design)
- Claude API key exposure (user responsibility to secure environment variables)
