# graphify-rs

[graphify](https://github.com/safishamsi/graphify) 的 Rust 重写版 — AI 驱动的知识图谱构建工具，将代码、文档、论文和图片转化为可查询的交互式知识图谱。

[English](README.md)

## 与 Python 原版的区别

**功能完全对等**，另有以下改进：

| 方面 | Python（原版）| Rust（本仓库）|
|------|-------------|-------------|
| 性能 | ~204ms, ~48MB 内存 | ~24ms, ~1MB 内存（快 8.5 倍，内存少 48 倍）|
| AST 解析 | 仅正则 | 10 种语言原生 tree-sitter + 正则回退 |
| 语义提取 | 串行 | 并发，可配置并行数（`-j`）|
| MCP 服务器 | 无 | 7 个工具，JSON-RPC 2.0 stdio |
| 导出格式 | 7 种 | 8 种（+ Obsidian 知识库）|
| CLI | 基础 | 21 个子命令、`--quiet`/`--verbose`、Shell 补全 |
| 进度反馈 | 无 | 大项目提取时显示进度条 |
| 配置 | 仅命令行 | `graphify.toml` 项目级默认配置 |
| Watch 模式 | 全量重建 | 增量重建（仅变更文件重新提取）|
| 图谱对比 | 仅函数 | `graphify-rs diff` 命令，彩色输出 |
| 图谱统计 | 无 | `graphify-rs stats` 独立命令 |
| 终端输出 | 纯文本 | 彩色输出 |

输出格式**完全兼容** — `graph.json` 使用相同的 NetworkX `node_link_data` 格式，Python 工具可直接读取 Rust 输出，反之亦然。

## 快速开始

```bash
cargo install --path .
graphify-rs build
open graphify-out/graph.html
```

## 使用方法

```bash
# 构建
graphify-rs build --path . --output graphify-out
graphify-rs build --format json,html,report    # 选择导出格式
graphify-rs build --code-only                   # 仅处理代码文件
graphify-rs build --update                      # 增量重建
graphify-rs build --no-llm                      # 跳过 Claude API

# 全局参数
graphify-rs -q build                            # 安静模式
graphify-rs -v build                            # 详细/调试模式
graphify-rs -j 4 build                          # 限制并行任务数

# 查询与分析
graphify-rs query "认证是如何工作的"
graphify-rs diff old/graph.json new/graph.json
graphify-rs stats graphify-out/graph.json

# MCP 服务器（Claude Code 集成）
graphify-rs serve --graph graphify-out/graph.json

# 文件监控与自动重建
graphify-rs watch --path . --output graphify-out

# 抓取 URL 内容
graphify-rs ingest https://arxiv.org/abs/2301.00001

# Git 钩子
graphify-rs hook install

# 平台集成
graphify-rs claude install
graphify-rs codex install

# Shell 补全
graphify-rs completions bash > ~/.bash_completion.d/graphify-rs

# 配置文件
graphify-rs init                                # 生成 graphify.toml
```

## 配置文件

在项目根目录创建 `graphify.toml`（或运行 `graphify-rs init`）：

```toml
output = "graphify-out"
no_llm = false
code_only = false
formats = ["json", "html", "report"]
```

CLI 参数始终覆盖配置文件中的值。

## 架构

14 个 crate 组成 Cargo workspace：

| Crate | 用途 |
|-------|------|
| `graphify-core` | 数据模型、图操作、ID 生成、置信度体系 |
| `graphify-detect` | 文件发现、分类、.graphifyignore、敏感文件过滤 |
| `graphify-extract` | AST 提取（tree-sitter + 正则）、Claude API 语义提取 |
| `graphify-build` | 图组装、去重 |
| `graphify-cluster` | 社区检测（Louvain）、凝聚力评分 |
| `graphify-analyze` | 高连接节点、跨社区惊奇连接、建议问题、图差异 |
| `graphify-export` | JSON, HTML, SVG, GraphML, Cypher, Wiki, 报告, Obsidian |
| `graphify-cache` | SHA256 内容哈希缓存，原子写入 |
| `graphify-security` | URL/路径/标签校验、SSRF 防御 |
| `graphify-ingest` | URL 抓取（arXiv, 推文, PDF, 网页）|
| `graphify-serve` | MCP 服务器（7 个工具）、BFS/DFS 遍历、评分 |
| `graphify-watch` | 文件监控 + debounce、增量重建 |
| `graphify-hooks` | Git 钩子安装/卸载/状态 |
| `graphify-benchmark` | Token 效率指标 |

## 导出格式

| 文件 | 说明 |
|------|------|
| `graph.json` | 兼容 NetworkX node_link_data 的 JSON |
| `graph.html` | vis.js 交互式可视化（暗色主题）|
| `GRAPH_REPORT.md` | 分析报告：社区、高连接节点、惊奇连接 |
| `graph.svg` | 静态图谱可视化 |
| `graph.graphml` | 适用于 yEd、Gephi 等图编辑器 |
| `cypher.txt` | Neo4j 导入脚本 |
| `wiki/` | 按社区组织的 Wiki 页面 |
| `obsidian/` | 带 wikilinks 的 Obsidian 知识库 |

## MCP 服务器工具

运行 `graphify-rs serve` 后，通过 JSON-RPC 2.0（stdio）提供 7 个工具：

| 工具 | 说明 |
|------|------|
| `query_graph` | 按关键词搜索节点，返回子图上下文 |
| `get_node` | 获取特定节点的详细信息 |
| `get_neighbors` | 获取节点的邻居和连接边 |
| `get_community` | 列出社区中的所有节点 |
| `god_nodes` | 查找最高连接度的中心节点 |
| `graph_stats` | 图谱整体统计 |
| `shortest_path` | 查找两个节点之间的最短路径 |

## 支持的语言

| 原生（tree-sitter）| 正则回退 |
|-------------------|---------|
| Python, JavaScript, TypeScript, Rust, Go | PHP, Swift, Kotlin, Scala, Dart |
| Java, C, C++, Ruby, C# | Lua, Haskell, Elixir, Shell/Bash, R |

## 许可证

MIT — 详见 [LICENSE](LICENSE)。

本项目是 [graphify](https://github.com/safishamsi/graphify)（作者 safishamsi）的 Rust 重写版。
