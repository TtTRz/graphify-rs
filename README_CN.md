# graphify-rs

[![Crates.io](https://img.shields.io/crates/v/graphify-rs.svg)](https://crates.io/crates/graphify-rs)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

AI 驱动的知识图谱构建工具 — 将代码、文档、论文和图片转化为可查询的交互式知识图谱。

[English](README.md) | [CLI 参考](docs/CLI_CN.md) | [更新日志](CHANGELOG.md)

## 什么是 graphify-rs？

**graphify-rs** 基于 [Andrej Karpathy 的 /raw 文件夹工作流](https://x.com/karpathy/status/1871129915774632404)：把任何东西丢进一个文件夹 — 论文、推文、截图、代码、笔记 — 就能得到一个结构化的知识图谱，发现你从未想到的跨文档关联。

它是 [graphify](https://github.com/safishamsi/graphify)（Python）的 Rust 完全重写版，功能完全对等且性能大幅提升。

### LLM 做不到的三件事

1. **持久化图谱** — 关系存储在 `graph.json` 中，跨会话存活。几周后提问也无需重新阅读所有文件。
2. **诚实的审计链** — 每条边都标记为 `EXTRACTED`（确定性提取）、`INFERRED`（推断）或 `AMBIGUOUS`（模糊）。你永远知道哪些是从源码中找到的，哪些是推断出来的。
3. **跨文档惊喜** — 社区检测能发现不同文件中的概念之间的联系，这些联系你永远不会想到去直接询问。

### 使用场景

- **初识代码库** — 在动手之前先理解架构
- **研究语料库** — 论文 + 推文 + 笔记 → 一个可导航的图谱，包含引用和概念边
- **个人 /raw 文件夹** — 什么都丢进去，让它自然生长，随时查询
- **Agent 工作流** — AI Agent 通过 MCP 服务器查询图谱，获取结构化的上下文

## 与 Python 原版的区别

| 方面 | Python（原版）| Rust（本仓库）|
|------|-------------|-------------|
| 性能 | ~204ms, ~48MB 内存 | ~24ms, ~1MB 内存（快 8.5 倍，内存少 48 倍）|
| AST 解析 | 仅正则 | 11 种语言原生 tree-sitter + 正则回退 |
| 语义提取 | 串行 | 并发，可配置并行数（`-j`）|
| 社区检测 | Louvain (graspologic) | Leiden（手写实现，带细化阶段）|
| MCP 服务器 | 无 | 7 个工具，JSON-RPC 2.0 stdio |
| 导出格式 | 7 种 | 9 种（+ Obsidian 知识库、按社区拆分 HTML）|
| CLI | 基础 | 21 个子命令、`--quiet`/`--verbose`、Shell 补全 |
| Watch 模式 | 全量重建 | 增量重建（仅变更文件重新提取）|

输出格式**完全兼容** — `graph.json` 使用相同的 NetworkX `node_link_data` 格式。

## 安装

### 从 crates.io 安装

```bash
cargo install graphify-rs
```

### 从源码安装

```bash
git clone https://github.com/TtTRz/graphify-rs.git
cd graphify-rs
cargo install --path .
```

## 快速开始

```bash
graphify-rs build                        # 从当前目录构建知识图谱
open graphify-out/graph.html             # 在浏览器中探索
graphify-rs query "认证是如何工作的？"     # 查询图谱
```

完整 CLI 参考请见 **[docs/CLI_CN.md](docs/CLI_CN.md)**。

## 工作原理

### 管线概览

```
 源文件                graphify-rs build
 ┌──────────┐    ┌─────────────────────────────────────────────────────────┐
 │ .py .rs  │    │                                                         │
 │ .go .ts  │───▶│  发现 → 提取 → 构建 → 聚类 → 分析 → 导出               │
 │ .md .pdf │    │                                                         │
 │ .png     │    └──────────┬──────────────────────────────────────────────┘
 └──────────┘               │
                            ▼
                  graphify-out/
                  ├── graph.json        （可查询的图谱数据）
                  ├── graph.html        （交互式可视化）
                  ├── GRAPH_REPORT.md   （分析报告）
                  ├── wiki/             （按社区组织的 Wiki）
                  └── obsidian/         （Obsidian 知识库）
```

### 两轮提取

**第 1 轮 — 确定性 AST 提取**（免费、快速、始终运行）：

使用 [tree-sitter](https://tree-sitter.github.io/) 将源代码解析为 AST，然后提取函数、类、导入和调用关系。支持 21 种语言，其中 11 种有原生 tree-sitter 语法，其余走正则回退。此轮的每条边标记为 `EXTRACTED`，置信度 1.0。

**第 2 轮 — Claude API 语义提取**（可选，`--no-llm` 跳过）：

将文档/论文/图片内容发送给 Claude API，发现语法本身无法揭示的高层关系 — 概念关联、共享假设、设计意图。此轮的边标记为 `INFERRED`，置信度 0.4–0.9。

### 置信度体系

图谱中每条边都带有置信度标签：

| 标签 | 含义 | 分数 |
|------|------|------|
| `EXTRACTED` | 直接从源码中找到（import、调用、引用）| 1.0 |
| `INFERRED` | 从上下文合理推断 | 0.4–0.9 |
| `AMBIGUOUS` | 不确定 — 标记供人工审查 | 0.1–0.3 |

确保你始终知道哪些关系是事实，哪些是猜测。

### Leiden 社区检测

构建图谱后，graphify-rs 运行 [Leiden 算法](https://www.nature.com/articles/s41598-019-41695-z)将节点划分为社区：

1. **Louvain 阶段** — 贪心模块度优化，将节点移动到能获得最大模块度增益的邻近社区
2. **细化阶段** — 在每个社区内进行 BFS 确保内部连通性；不连通的子社区被拆分
3. **小社区合并** — 少于 5 个节点的社区合并到其连接最紧密的邻居社区

每个社区获得一个凝聚力分数（实际社区内边数 / 最大可能边数），报告会展示"God Nodes"（最高连接度枢纽）和"惊奇连接"（桥接不同社区的边）。

## 架构

14 个 crate 组成 Cargo workspace：

| Crate | 用途 |
|-------|------|
| `graphify-core` | 数据模型（`GraphNode`, `GraphEdge`, `KnowledgeGraph`）、ID 生成、置信度体系 |
| `graphify-detect` | 文件发现、分类（code/doc/paper/image）、`.graphifyignore`、敏感文件过滤 |
| `graphify-extract` | AST 提取（tree-sitter，21 种语言）、Claude API 语义提取、去重 |
| `graphify-build` | 从提取结果组装图谱、节点/边去重 |
| `graphify-cluster` | Leiden 社区检测、凝聚力评分、社区拆分/合并 |
| `graphify-analyze` | 高连接节点、跨社区惊奇连接、建议问题、图谱 diff |
| `graphify-export` | 9 种格式：JSON, HTML, 拆分 HTML, SVG, GraphML, Cypher, Wiki, 报告, Obsidian |
| `graphify-cache` | SHA256 内容哈希缓存，支持增量重建 |
| `graphify-security` | URL 校验（SSRF 防御）、路径遍历防护、标签注入防御 |
| `graphify-ingest` | URL 抓取：arXiv 摘要、推文（oEmbed）、PDF、通用网页 |
| `graphify-serve` | MCP 服务器，7 个查询工具，JSON-RPC 2.0 stdio |
| `graphify-watch` | 文件监控 + debounce、代码变更时增量重建 |
| `graphify-hooks` | Git 钩子安装/卸载/状态（post-commit, post-checkout）|
| `graphify-benchmark` | Token 效率测量（图谱 token vs 原始语料 token）|

## 导出格式

| 文件 | 说明 |
|------|------|
| `graph.json` | 兼容 NetworkX `node_link_data` 的 JSON |
| `graph.html` | vis.js 交互式可视化（暗色主题，大图自动裁剪）|
| `html/` | 按社区拆分的 HTML 页面，带导航概览 |
| `GRAPH_REPORT.md` | 分析报告：社区、God Nodes、惊奇连接、建议问题 |
| `graph.svg` | 静态环形布局图谱可视化 |
| `graph.graphml` | 适用于 yEd、Gephi 等图编辑器 |
| `cypher.txt` | Neo4j Cypher 导入脚本 |
| `wiki/` | 按社区组织的 Wiki 风格 Markdown 页面 |
| `obsidian/` | 带 wikilinks 和 frontmatter 的 Obsidian 知识库 |

## CLI 参考

详见 **[docs/CLI_CN.md](docs/CLI_CN.md)**，包含所有命令、参数和示例。

快速概览：

```bash
graphify-rs build [--path .] [--no-llm] [--format json,html]  # 构建图谱
graphify-rs query "问题" [--dfs] [--budget 2000]               # 查询图谱
graphify-rs watch --path .                                      # 自动重建
graphify-rs serve                                                # MCP 服务器
graphify-rs diff old.json new.json                              # 图谱对比
graphify-rs stats graph.json                                    # 统计信息
```

## Agent 集成

graphify-rs 通过 skill 安装和 MCP 服务器与 AI 编码 Agent（Claude Code、Codex、OpenCode 等）集成。

```bash
graphify-rs install                # 全局安装 skill
graphify-rs claude install         # 项目级：CLAUDE.md + PreToolUse 钩子
graphify-rs serve                  # 启动 MCP 服务器供 Agent 查询
```

安装后，Agent 会自动在回答架构问题前检查知识图谱，并在代码修改后重建图谱。

完整 Agent 集成说明请见 CLI 参考中的 [Agent 集成](docs/CLI_CN.md#agent-集成)章节。

### MCP 服务器工具

| 工具 | 说明 |
|------|------|
| `query_graph` | 按关键词搜索节点，返回子图上下文 |
| `get_node` | 获取特定节点的详细信息 |
| `get_neighbors` | 获取节点的邻居和连接边 |
| `get_community` | 列出社区中的所有节点 |
| `god_nodes` | 查找最高连接度的中心节点 |
| `graph_stats` | 图谱整体统计 |
| `shortest_path` | 查找两个节点之间的最短路径 |

## 支持的语言（21 种）

| 原生（tree-sitter）| 正则回退 |
|-------------------|---------|
| Python, JavaScript, TypeScript, Rust, Go, Java | Kotlin, Scala, PHP, Swift, Lua |
| C, C++, Ruby, C#, Dart | Zig, PowerShell, Elixir, Obj-C, Julia |

## 参与贡献

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

MIT — 详见 [LICENSE](LICENSE)。

本项目是 [graphify](https://github.com/safishamsi/graphify)（作者 safishamsi）的 Rust 重写版。
