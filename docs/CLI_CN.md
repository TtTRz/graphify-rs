# CLI 参考手册

`graphify-rs` 是一个 AI 驱动的知识图谱构建工具，能够将代码、文档、论文和图片转化为可查询的交互式知识图谱。

## 全局参数

以下参数可用于**任何**子命令。

| 参数 | 缩写 | 类型 | 默认值 | 说明 |
|------|------|------|--------|------|
| `--quiet` | `-q` | `bool` | `false` | 静默模式，仅输出错误信息。 |
| `--verbose` | `-v` | `bool` | `false` | 详细输出模式（debug 级别），日志过滤器设为 `debug`。 |
| `--jobs <N>` | `-j` | `usize` | CPU 核心数 | 并行任务数。控制 rayon 线程池大小和语义提取并发数。 |

```bash
graphify-rs -q build                    # 静默构建
graphify-rs -v build                    # 调试输出
graphify-rs -j 4 build                  # 限制为 4 个线程
graphify-rs -q -j 2 serve               # 静默模式，2 个线程
```

---

## 命令

### `graphify-rs build`

从目录中的文件构建知识图谱。这是主要的处理流程：检测文件 -> AST 提取（第一遍）-> Claude API 语义提取（第二遍）-> 构建图谱 -> 社区聚类 -> 分析 -> 导出。

#### 参数

| 参数 | 缩写 | 类型 | 默认值 | 说明 |
|------|------|------|--------|------|
| `--path <PATH>` | `-p` | `String` | `"."` | 扫描源文件的根目录。 |
| `--output <DIR>` | `-o` | `String` | `"graphify-out"` | 所有生成文件的输出目录。 |
| `--no-llm` | | `bool` | `false` | 跳过 Claude API 语义提取（第二遍），仅运行 AST 提取。 |
| `--code-only` | | `bool` | `false` | 仅处理代码文件，跳过文档和论文。 |
| `--update` | | `bool` | `false` | 增量重建：仅重新提取自上次构建以来新增/修改的文件。 |
| `--format <FMT,...>` | | `String`（逗号分隔） | 所有格式 | 要生成的导出格式。可选：`json`、`html`、`graphml`、`cypher`、`svg`、`wiki`、`obsidian`、`report`。 |

#### 示例

```bash
# 完整构建当前目录，导出所有格式
graphify-rs build

# 构建指定项目，输出到自定义目录
graphify-rs build --path /path/to/project --output my-graph

# 快速 AST-only 构建（不调用 Claude API）
graphify-rs build --no-llm

# 仅处理代码文件，跳过文档和论文
graphify-rs build --code-only

# 编辑几个文件后增量重建
graphify-rs build --update

# 只生成 JSON 和 HTML
graphify-rs build --format json,html

# 只生成报告
graphify-rs build --format report

# 组合使用：快速增量、仅代码、JSON+报告
graphify-rs build --update --code-only --no-llm --format json,report
```

#### 构建流程

1. **检测** — 扫描 `--path` 目录中的代码、文档、论文和图片文件（遵循 `.graphifyignore`，跳过敏感文件）。
2. **AST 提取（第一遍）** — 对代码文件进行确定性的 tree-sitter + 正则提取。按文件 SHA256 缓存于 `<output>/cache/`。
3. **语义提取（第二遍）** — 对文档/论文进行并发 Claude API 提取（使用 `--no-llm` 或 `--code-only` 时跳过）。需要设置 `ANTHROPIC_API_KEY` 环境变量。并发数 = `min(--jobs, 8)`，默认 4。
4. **构建图谱** — 组装节点和边，去重。
5. **社区聚类** — Leiden 社区检测 + 内聚度评分。
6. **分析** — God 节点、意外连接、建议问题。
7. **导出** — 将选定格式写入 `--output`。

---

### `graphify-rs query`

使用自然语言查询知识图谱，返回子图上下文文本。

#### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `<QUESTION>`（位置参数） | `String` | *必填* | 自然语言查询问题。 |
| `--dfs` | `bool` | `false` | 使用深度优先搜索代替广度优先搜索进行遍历。 |
| `--budget <N>` | `usize` | `2000` | 输出文本的最大 token 预算。 |
| `--graph <PATH>` | `String` | `"graphify-out/graph.json"` | 图谱 JSON 文件路径。 |

#### 示例

```bash
# 基本查询
graphify-rs query "认证是如何工作的？"

# DFS 遍历，更大的 token 预算
graphify-rs query "错误处理流程" --dfs --budget 3000

# 查询指定图谱文件
graphify-rs query "数据库连接" --graph /path/to/graph.json
```

---

### `graphify-rs diff`

比较两个图谱快照，显示差异（新增/删除的节点和边）。

#### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `<OLD>`（位置参数） | `String` | *必填* | 旧版 `graph.json` 的路径。 |
| `<NEW>`（位置参数） | `String` | *必填* | 新版 `graph.json` 的路径。 |
| `--output <FORMAT>` | `String` | `"text"` | 输出格式：`text`（彩色终端）或 `json`。 |

#### 示例

```bash
# 比较两个图谱版本（彩色文本输出）
graphify-rs diff old-graph/graph.json new-graph/graph.json

# 输出 JSON 供程序使用
graphify-rs diff v1/graph.json v2/graph.json --output json
```

---

### `graphify-rs stats`

显示图谱统计信息，无需重新构建。显示节点/边数量、社区数、度分布、节点类型、边关系和最高连接度节点。

#### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `<GRAPH>`（位置参数） | `String` | `"graphify-out/graph.json"` | 图谱 JSON 文件路径。 |

#### 示例

```bash
# 默认图谱的统计信息
graphify-rs stats

# 指定图谱文件的统计信息
graphify-rs stats /path/to/graph.json
```

---

### `graphify-rs watch`

监视目录中的文件变更并自动增量重建图谱。

#### 参数

| 参数 | 缩写 | 类型 | 默认值 | 说明 |
|------|------|------|--------|------|
| `--path <PATH>` | `-p` | `String` | `"."` | 要监视的目录。 |
| `--output <DIR>` | `-o` | `String` | `"graphify-out"` | 图谱文件输出目录。 |

#### 示例

```bash
# 监视当前目录
graphify-rs watch

# 监视指定目录
graphify-rs watch --path src --output my-graph
```

---

### `graphify-rs serve`

启动 MCP（Model Context Protocol）服务器，通过 JSON-RPC 2.0（stdio）提供服务。提供 7 个 AI 智能体可直接调用的工具。

#### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `--graph <PATH>` | `String` | `"graphify-out/graph.json"` | 要提供服务的图谱 JSON 文件路径。 |

#### 可用的 MCP 工具

| 工具 | 说明 |
|------|------|
| `query_graph` | 按关键词搜索节点，返回子图上下文 |
| `get_node` | 获取指定节点的详细信息 |
| `get_neighbors` | 获取节点的邻居和连接边 |
| `get_community` | 列出社区中的所有节点 |
| `god_nodes` | 查找连接度最高的枢纽节点 |
| `graph_stats` | 图谱整体统计信息 |
| `shortest_path` | 查找两个节点之间的最短路径 |

#### 示例

```bash
# 使用默认图谱启动 MCP 服务器
graphify-rs serve

# 为指定图谱提供服务
graphify-rs serve --graph /path/to/graph.json
```

---

### `graphify-rs ingest`

从 URL 抓取内容（arXiv 论文、推文、PDF、网页）并添加到图谱输出目录。

#### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `<URL>`（位置参数） | `String` | *必填* | 要抓取内容的 URL。 |
| `--output <DIR>` | `-o` | `String` | `"graphify-out"` | 输出目录。 |

#### 示例

```bash
# 抓取 arXiv 论文
graphify-rs ingest https://arxiv.org/abs/2301.00001

# 抓取网页到自定义输出目录
graphify-rs ingest https://example.com/docs --output my-graph
```

---

### `graphify-rs hook`

Git 钩子管理。安装、卸载或查看 git 钩子状态，钩子会在提交时自动重建图谱。

#### 子命令

| 子命令 | 说明 |
|--------|------|
| `install` | 安装 git 钩子（pre-commit）。 |
| `uninstall` | 移除已安装的 git 钩子。 |
| `status` | 显示当前钩子安装状态。 |

#### 示例

```bash
graphify-rs hook install      # 安装 pre-commit 钩子
graphify-rs hook uninstall    # 移除钩子
graphify-rs hook status       # 检查钩子是否已安装
```

---

### `graphify-rs claude install` / `uninstall`

项目级 Claude Code 集成。安装 `PreToolUse` 钩子并将图谱指令添加到 `CLAUDE.md`。

#### `install` 做了什么

1. 在 `./CLAUDE.md` 中追加 `## graphify` 章节，包含智能体读取图谱报告的规则。
2. 在 `.claude/settings.json` 中写入 `PreToolUse` 钩子，在 `Glob|Grep` 工具调用时触发。

#### `uninstall` 做了什么

1. 从 `./CLAUDE.md` 中移除 `## graphify` 章节。
2. 从 `.claude/settings.json` 中移除钩子。

#### 示例

```bash
graphify-rs claude install
graphify-rs claude uninstall
```

---

### `graphify-rs codex install` / `uninstall`

项目级 Codex 集成。将钩子写入 `.codex/hooks.json`，并将指令添加到 `AGENTS.md`。

#### 示例

```bash
graphify-rs codex install
graphify-rs codex uninstall
```

---

### `graphify-rs opencode install` / `uninstall`

项目级 OpenCode 集成。将插件写入 `.opencode/plugins/graphify.js`，在 `opencode.json` 中注册，并将指令添加到 `AGENTS.md`。

#### 示例

```bash
graphify-rs opencode install
graphify-rs opencode uninstall
```

---

### `graphify-rs claw install` / `uninstall`

项目级 OpenClaw 集成。将图谱指令添加到 `AGENTS.md`。

#### 示例

```bash
graphify-rs claw install
graphify-rs claw uninstall
```

---

### `graphify-rs droid install` / `uninstall`

项目级 Factory Droid 集成。将图谱指令添加到 `AGENTS.md`。

#### 示例

```bash
graphify-rs droid install
graphify-rs droid uninstall
```

---

### `graphify-rs trae install` / `uninstall`

项目级 Trae 集成。将图谱指令添加到 `AGENTS.md`。

#### 示例

```bash
graphify-rs trae install
graphify-rs trae uninstall
```

---

### `graphify-rs trae-cn install` / `uninstall`

项目级 Trae CN 集成。将图谱指令添加到 `AGENTS.md`。

#### 示例

```bash
graphify-rs trae-cn install
graphify-rs trae-cn uninstall
```

---

### `graphify-rs install`

为 AI 编码助手平台全局安装 graphify 技能。将 `SKILL.md` 文件写入平台的技能目录并在平台配置中注册。

#### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `--platform <NAME>` | `String` | `"claude"` | 要安装的平台。可选值：`claude`、`codex`、`opencode`、`claw`、`droid`、`trae`、`trae-cn`、`windows`。 |

#### 技能文件位置

| 平台 | 技能路径 |
|------|----------|
| `claude` | `~/.claude/skills/graphify/SKILL.md` |
| `codex` | `~/.agents/skills/graphify/SKILL.md` |
| `opencode` | `~/.config/opencode/skills/graphify/SKILL.md` |
| `claw` | `~/.claw/skills/graphify/SKILL.md` |
| `droid` | `~/.factory/skills/graphify/SKILL.md` |
| `trae` | `~/.trae/skills/graphify/SKILL.md` |
| `trae-cn` | `~/.trae-cn/skills/graphify/SKILL.md` |
| `windows` | `~/.claude/skills/graphify/SKILL.md` |

#### 示例

```bash
# 安装到 Claude（默认）
graphify-rs install

# 安装到 Codex
graphify-rs install --platform codex

# 安装到 OpenCode
graphify-rs install --platform opencode
```

---

### `graphify-rs init`

在当前目录初始化 `graphify.toml` 配置文件，包含注释掉的默认值。如果文件已存在则会失败。

#### 示例

```bash
graphify-rs init
```

生成的文件：

```toml
# graphify-rs configuration
# These values serve as defaults and can be overridden by CLI flags.

# Output directory for graph files
# output = "graphify-out"

# Disable LLM-based semantic extraction
# no_llm = false

# Only process code files (skip docs/papers)
# code_only = false

# Export formats (comma-separated). Available: json,html,graphml,cypher,svg,wiki,obsidian,report
# Leave empty or omit for all formats.
# formats = ["json", "html", "report"]
```

---

### `graphify-rs completions`

生成 Shell 补全脚本。

#### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `<SHELL>`（位置参数） | `Shell` | *必填* | 要生成补全的 Shell。可选值：`bash`、`zsh`、`fish`、`elvish`、`powershell`。 |

#### 示例

```bash
# Bash
graphify-rs completions bash > ~/.bash_completion.d/graphify-rs

# Zsh
graphify-rs completions zsh > ~/.zfunc/_graphify-rs

# Fish
graphify-rs completions fish > ~/.config/fish/completions/graphify-rs.fish

# PowerShell
graphify-rs completions powershell > graphify-rs.ps1
```

---

### `graphify-rs benchmark`

对图谱文件运行 token 效率基准测试。

#### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `<GRAPH_PATH>`（位置参数） | `String` | `"graphify-out/graph.json"` | 图谱 JSON 文件路径。 |

#### 示例

```bash
# 对默认图谱进行基准测试
graphify-rs benchmark

# 对指定图谱进行基准测试
graphify-rs benchmark /path/to/graph.json
```

---

### `graphify-rs save-result`

将查询结果保存到记忆目录以供将来参考。

#### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `--question <TEXT>` | `String` | *必填* | 提问的问题。 |
| `--answer <TEXT>` | `String` | *必填* | 生成的回答。 |
| `--type <TYPE>` | `String` | `"query"` | 结果类型标识。 |
| `--nodes <ID>...` | `Vec<String>` | `[]` | 相关的节点 ID（可多次指定）。 |
| `--memory-dir <DIR>` | `String` | `"graphify-out/memory"` | 保存结果的目录。 |

#### 示例

```bash
# 保存查询结果
graphify-rs save-result \
  --question "认证是怎么工作的？" \
  --answer "认证使用 JWT 令牌，通过 auth 模块..." \
  --type query \
  --nodes auth_module --nodes jwt_handler

# 保存到自定义记忆目录
graphify-rs save-result \
  --question "数据库架构" \
  --answer "使用 PostgreSQL，包含 12 张表..." \
  --memory-dir my-graph/memory
```

---

## 配置文件（`graphify.toml`）

在项目根目录创建 `graphify.toml` 文件（或运行 `graphify-rs init`）以设置项目级默认值。

### 字段

| 字段 | 类型 | 默认值 | CLI 覆盖参数 | 说明 |
|------|------|--------|-------------|------|
| `output` | `String` | `"graphify-out"` | `--output` | 图谱文件输出目录。 |
| `no_llm` | `bool` | `false` | `--no-llm` | 禁用基于 LLM 的语义提取。 |
| `code_only` | `bool` | `false` | `--code-only` | 仅处理代码文件（跳过文档/论文）。 |
| `formats` | `String[]` | `[]`（所有格式） | `--format` | 要生成的导出格式。 |

### 优先级规则

1. **CLI 参数**始终具有最高优先级。
2. **`graphify.toml`** 中的值作为 CLI 参数未设置时的默认值。
3. **内置默认值**在 CLI 和配置文件都未指定时使用。

具体的合并规则：
- `output`：如果 CLI 值与内置默认值（`"graphify-out"`）不同则使用 CLI 值；否则回退到配置文件。
- `no_llm`：如果 CLI 参数**或**配置文件中**任一**为 `true` 则为 `true`（OR 逻辑）。
- `code_only`：如果 CLI 参数**或**配置文件中**任一**为 `true` 则为 `true`（OR 逻辑）。
- `formats`：如果 CLI 值非空则使用 CLI 值；否则回退到配置文件。空值表示所有格式。

### 示例

```toml
# 始终输出到自定义目录
output = "knowledge-graph"

# 默认跳过 Claude API 调用
no_llm = true

# 只生成 JSON 和 HTML
formats = ["json", "html"]
```

### 环境变量

| 变量 | 说明 |
|------|------|
| `ANTHROPIC_API_KEY` | 语义提取（第二遍）所需。没有此变量时，文档/论文文件仅运行 AST 提取。 |
| `RUST_LOG` | 日志级别过滤器（默认：`warn`）。被 `-v`（`debug`）或 `-q`（`error`）覆盖。 |

---

## 智能体集成

将 `graphify-rs` 作为 AI 编码智能体技能的完整设置指南。

### 平台设置

#### Claude Code

```bash
# 1. 安装项目级集成
graphify-rs claude install

# 2. 构建图谱
graphify-rs build

# 3.（可选）安装全局技能以使用 /graphify 斜杠命令
graphify-rs install --platform claude
```

`claude install` 创建的内容：
- `./CLAUDE.md` — 追加 `## graphify` 章节，包含智能体规则
- `.claude/settings.json` — 添加 `PreToolUse` 钩子，在 `Glob|Grep` 工具调用时触发，提醒智能体先查看图谱

#### Codex

```bash
# 1. 安装项目级集成
graphify-rs codex install

# 2. 构建图谱
graphify-rs build

# 3.（可选）安装全局技能
graphify-rs install --platform codex
```

`codex install` 创建的内容：
- `./AGENTS.md` — 追加 `## graphify` 章节，包含智能体规则
- `.codex/hooks.json` — 添加 `PreToolUse` 钩子，在 `Bash` 工具调用时触发

#### OpenCode

```bash
# 1. 安装项目级集成
graphify-rs opencode install

# 2. 构建图谱
graphify-rs build

# 3.（可选）安装全局技能
graphify-rs install --platform opencode
```

`opencode install` 创建的内容：
- `./AGENTS.md` — 追加 `## graphify` 章节，包含智能体规则
- `.opencode/plugins/graphify.js` — PreToolUse 插件
- `opencode.json` — 注册插件

#### Claw / Droid / Trae / Trae CN

```bash
graphify-rs claw install       # 或 droid、trae、trae-cn
graphify-rs build
```

这些平台使用通用集成，仅将 `## graphify` 章节写入 `./AGENTS.md`。

### 智能体如何使用图谱

安装后，智能体遵循以下规则（注入到 `CLAUDE.md` 或 `AGENTS.md`）：

1. **在回答架构或代码库问题之前** — 读取 `graphify-out/GRAPH_REPORT.md` 了解 God 节点和社区结构。
2. **如果 `graphify-out/wiki/index.md` 存在** — 浏览 wiki 而不是读取原始文件。
3. **对于具体问题** — 运行 `graphify-rs query "<问题>"` 获取相关子图上下文。
4. **修改代码文件后** — 运行 `graphify-rs build --path . --output graphify-out --no-llm --update` 保持图谱最新（快速，仅 AST，约 2-5 秒）。

`PreToolUse` 钩子会在智能体使用 `Glob` 或 `Grep` 工具（Claude）或 `Bash`（Codex）时自动触发，注入提醒智能体先查看图谱的消息。

### MCP 服务器集成

要实现更深层的集成，运行 MCP 服务器，使智能体可以直接调用图谱工具。

#### Claude Desktop 配置

添加到 Claude Desktop MCP 配置（`claude_desktop_config.json`）：

```json
{
  "mcpServers": {
    "graphify": {
      "command": "graphify-rs",
      "args": ["serve", "--graph", "graphify-out/graph.json"]
    }
  }
}
```

#### Claude Code MCP 配置

添加到 `.claude/settings.json`：

```json
{
  "mcpServers": {
    "graphify": {
      "command": "graphify-rs",
      "args": ["serve", "--graph", "graphify-out/graph.json"]
    }
  }
}
```

智能体随后可以通过 MCP 协议直接调用 `query_graph`、`get_node`、`get_neighbors`、`god_nodes`、`graph_stats`、`get_community` 和 `shortest_path` 等工具。

### 代码变更后保持图谱最新

```bash
# 快速增量重建（仅 AST，约 2-5 秒）
graphify-rs build --no-llm --update

# 或使用监视模式自动重建
graphify-rs watch

# 或安装 git 钩子，在提交时重建
graphify-rs hook install
```

### 版本过期检测

`graphify-rs` 在每次调用时检查技能文件版本。如果已安装的技能由不同版本的 `graphify-rs` 写入，会打印警告：

```
warning: skill is from graphify-rs 0.2.0, package is 0.3.0. Run 'graphify-rs install' to update.
```

运行 `graphify-rs install` 更新技能文件。
