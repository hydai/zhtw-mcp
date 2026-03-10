# zhtw-mcp

A linguistic linter for Traditional Chinese (zh-TW) that enforces Taiwan Ministry of Education (MoE) standards on vocabulary, punctuation, and character shapes. It plugs into AI coding assistants through the [Model Context Protocol](https://modelcontextprotocol.io/) (MCP) and catches Mainland Chinese (zh-CN) regional drift before it reaches the user.

The tool enforces three official Taiwan standards:

- [Revised Handbook of Punctuation](https://language.moe.gov.tw/001/upload/files/site_content/m0001/hau/c2.htm) (《重訂標點符號手冊》修訂版) -- punctuation marks
- [Standard Form of National Characters](https://language.moe.gov.tw/001/Upload/files/SITE_CONTENT/M0001/STD/F4.HTML) (《國字標準字體》) -- character shapes
- Cross-strait vocabulary normalization, grounded in [OpenCC](https://github.com/BYVoid/OpenCC)'s TWPhrases/TWVariants datasets -- word choices

Over 1000 vocabulary rules and 15 casing rules are compiled into the binary. For ambiguous terms, the server asks the AI assistant it runs inside for help deciding -- no extra API keys required.

## Why this exists

### Modern Chinese is an inadequately standardized language

In the late Qing dynasty, scholars had to express Western concepts in a writing system with no native vocabulary for them. Whether coining new words or importing translations via Japanese (和製漢語), they assembled a literary system under enormous time pressure. Many translated terms were inconsistent, ambiguous, or contradictory. The Chinese-speaking world has lived with these deficiencies for over a century.

### Simplified Chinese made it worse

The PRC simplification effort reduced not just stroke counts but vocabulary precision. Terms that should vary by domain got flattened into single catch-all translations. Many PRC translations were coined hastily: if a term worked in one context, it spread uncritically to others.

### AI models amplify the problem

AI language models learn from web text where Simplified Chinese vastly outweighs Traditional Chinese (roughly 2.6:1 in [CC-100](https://data.statmt.org/cc-100/)). Major datasets like [CulturaX](https://huggingface.co/datasets/uonlp/CulturaX) do not even track Traditional Chinese separately. A [FAccT 2025 study](https://arxiv.org/abs/2505.22645) confirmed that most models favor zh-CN terminology when asked to write zh-TW. The output looks plausible but is not how people in Taiwan actually write.

This goes beyond character conversion. The same word often means different things across the strait:

| English | zh-CN | zh-TW | Why it matters |
|---------|-------|-------|----------------|
| concurrency | 並發 | 並行 | In zh-CN, 並行 means "parallel" -- a different concept entirely |
| parallel | 並行 | 平行 | zh-CN 並行 = "parallel"; in Taiwan, 並行 = "concurrent" |
| process (OS) | 進程 | 行程 | 進程 in Taiwan means "progress," not an OS process |
| file / document | 文件 / 文檔 | 檔案 / 文件 | 文件 in China = "file"; in Taiwan = "document" |
| render | 渲染 | 算繪 | 渲染 in Taiwan = "exaggerate" (a painting technique) |
| traverse | 遍歷 | 走訪 | 遍歷 in Taiwan is reserved for Ergodic theory (遍歷理論) |

### What this project does

Automatically check and correct zh-TW text produced by AI, catching cross-strait terminology leaks:

- Half-width punctuation (`,` `.` `:`) that should be full-width (`，` `。` `：`)
- Mainland-style `""` curly quotes replaced with Taiwan-style `「」` corner brackets
- Missing or extra CJK-Latin/digit spacing
- Mainland vocabulary -- 軟件→軟體, 內存→記憶體, 默認→預設, etc.
- Non-standard character variants -- 裏→裡, 着→著 per MoE standard forms
- Politically colored terms -- 祖國, 內地
- Casing -- JavaScript, GitHub, macOS

Three profiles (`default`, `strict_moe`, `ui_strings`) control which rules apply. See [docs/rules.md](docs/rules.md) for the full rule reference.

## Naming convention: cn and tw

This project follows [BCP 47](https://www.rfc-editor.org/info/bcp47). The region subtag comes from [ISO 3166-1 alpha-2](https://www.iso.org/iso-3166-country-codes.html), where "region" can denote a sovereign state, territory, or economic area -- not necessarily a "country."

- `zh-CN`: Chinese as written in the CN region (Simplified)
- `zh-TW`: Chinese as written in the TW region (Traditional)

Throughout the codebase, `cn` and `tw` denote regional writing conventions, not a political statement.

## Getting started

### Building from source

Requires stable Rust 1.91+.

```bash
cargo build --release
```

The binary is at `target/release/zhtw-mcp`.

### Installing

The quickest way to build, install to `~/.local/bin`, and register with Claude Code:

```bash
make install      # build release, install binary, register MCP server
make uninstall    # remove binary and MCP registration
make status       # check binary, process, and registration state
```

For manual setup or other MCP clients:

```bash
# Claude Code
claude mcp add zhtw-mcp -- /path/to/zhtw-mcp

# OpenCode
opencode mcp add zhtw-mcp /path/to/zhtw-mcp
```

Codex CLI or other MCP clients -- add to `.mcp.json` in your project root:

```json
{
  "mcpServers": {
    "zhtw-mcp": {
      "command": "/path/to/zhtw-mcp",
      "args": []
    }
  }
}
```

Replace `/path/to/zhtw-mcp` with the actual binary path (e.g., `target/release/zhtw-mcp`).

### CLI quick start

```bash
zhtw-mcp lint README.md                 # lint a file
zhtw-mcp lint file.md --fix             # auto-fix in place
zhtw-mcp lint file.md --fix --dry-run   # preview fixes
```

See [docs/cli.md](docs/cli.md) for the full CLI reference and [docs/mcp.md](docs/mcp.md) for MCP tool/resource/prompt details.

### Common prompts

When running as an MCP server, you interact through natural language. The assistant translates your intent into `zhtw` tool calls.

| Intent | Say | Result |
|--------|-----|--------|
| Lint text | *"Check this paragraph for mainland terms"* | Returns issues with location and suggestions |
| Lint a file | *"Lint README.md for zh-TW correctness"* | Assistant reads file, passes text to `zhtw` |
| Auto-fix | *"Fix the zh-TW issues in this document"* | Safe fixes applied, corrected text returned |
| Aggressive fix | *"Aggressively fix all zh-TW problems"* | Context-aware fixes including ambiguous terms |
| Quality gate | *"Reject if more than 3 zh-TW errors"* | Accept/reject verdict via `max_errors` |
| Strict mode | *"Check this with strict MoE rules"* | Enables variant and full punctuation enforcement |
| Markdown-aware | *"Lint this markdown, skip code blocks"* | Excludes fenced code and HTML |
| Ignore terms | *"Check this text, but allow '程序'"* | Matching issues downgraded to Info severity |
| Explain issues | *"Explain each issue in this text"* | English explanations with MoE references |
| Political terms | *"Flag politically colored terms"* | Detects 祖國, 內地, etc. |
| UI localization | *"Lint this YAML file with ui_strings profile"* | YAML-aware scan, half-width colon allowed |
| Editorial review | *"Review and refine this zh-TW draft"* | Iterative review → fix → re-check cycles |

The server also exposes two read-only resources for assistants to consult: `zh-tw://style-guide/moe` (MoE standards) and `zh-tw://dictionary/ambiguous` (cross-strait term disambiguation).

## Extending the ruleset

### Adding a spelling rule

Edit `assets/ruleset.json`:

```json
{
  "from": "數據庫",
  "to": ["資料庫"],
  "type": "cross_strait",
  "context": "database = 資料庫",
  "english": "database"
}
```

Run `scripts/check-ruleset.py --lint` to validate before opening a PR.

Fields: `from` (required), `to` (required, array), `type` (required: `cross_strait` / `political_coloring` / `confusable` / `typo` / `variant`), `disabled` (optional), `context` (optional, use `@seealso` for cross-refs), `english` (optional, recommended).

### Adding a case rule

```json
{
  "term": "GraphQL",
  "alternatives": ["graphql", "GRAPHQL", "Graphql"]
}
```

### Runtime overrides

Edit `overrides.json` in the platform config directory (`~/.config/zhtw-mcp/` on Linux, `~/Library/Application Support/zhtw-mcp/` on macOS):

```json
{
  "schema_version": 3,
  "spelling": [
    {"from": "優化", "to": ["最佳化"], "type": "cross_strait", "disabled": true}
  ],
  "case": []
}
```

## Further reading

- [docs/cli.md](docs/cli.md) -- full CLI reference, config files, CI/CD integration, S2T conversion
- [docs/mcp.md](docs/mcp.md) -- MCP tool parameters, resources, prompts, sampling, usage examples
- [docs/internals.md](docs/internals.md) -- processing pipeline, script detection, design decisions, testing
- [docs/rules.md](docs/rules.md) -- rule type reference (cross-strait, punctuation, variant, political, case)

## License

`zhtw-mcp` is available under a permissive MIT-style license.
Use of this source code is governed by a MIT license that can be found in the [LICENSE](LICENSE) file.
