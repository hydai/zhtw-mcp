// Agentic Editor integration: generate host-specific configuration content.
//
// Supports three hosts:
//   - Claude Code: CLAUDE.md section for zh-TW grounding
//   - OpenCode:    Skill definition YAML
//   - Copilot:     copilot-instructions.md + VS Code MCP settings

/// Generate a CLAUDE.md section for Claude Code integration.
///
/// Embeds the normalize_tone prompt content and references zhtw-mcp tools
/// so Claude Code automatically enforces zh-TW conventions.
pub fn claude_code_section() -> String {
    r#"## zh-TW Writing Standards (zhtw-mcp)

Use `zhtw` to lint/fix/gate Traditional Chinese (Taiwan) text per MoE standards.
Read `zh-tw://style-guide/moe` resource for full conventions.

### Quick Reference

- Terms: 軟體 (not 軟件), 資訊 (not 信息), 預設 (not 默認)
- Punctuation: full-width ，。：；！？ in CJK prose; 「」 quotes, 『』 nested
- Profiles: `default` | `strict_moe` (char variants) | `ui_strings` (relaxed) | `editorial` (AI writing review)

### Quality Gate

```
zhtw({ "text": "...", "fix_mode": "lexical_safe", "max_errors": 0, "output": "compact" })
```

Re-run until `accepted: true`. Use `output: "compact"` to save context tokens."#
        .to_string()
}

/// Generate an OpenCode skill definition YAML.
pub fn opencode_skill() -> String {
    r#"# OpenCode Skill: zh-TW Text Linting
# Place in .opencode/skills/zhtw-lint.yaml

name: zhtw-lint
description: Lint and fix Traditional Chinese (Taiwan) text using MoE standards
trigger:
  # Activate when working with Chinese text files
  file_patterns:
    - "*.md"
    - "*.txt"
    - "*.rst"
  content_patterns:
    - "[\u4e00-\u9fff]"  # CJK Unified Ideographs

steps:
  - name: check
    tool: zhtw
    arguments:
      text: "{{content}}"
      fix_mode: "lexical_safe"
      max_errors: 0
      content_type: "{{if file_ext == 'md'}}markdown{{else}}plain{{end}}"
      profile: "default"

context:
  resources:
    - zh-tw://style-guide/moe
  prompts:
    - normalize_tone"#
        .to_string()
}

/// Generate GitHub Copilot integration instructions.
///
/// Returns a tuple of (copilot_instructions_md, vscode_settings_json_snippet).
pub fn copilot_config() -> (String, String) {
    let instructions = r#"# GitHub Copilot Instructions for zh-TW

When generating or editing Traditional Chinese (Taiwan) text in this project,
follow Ministry of Education (教育部) standards:

## Vocabulary
Use Taiwan-standard terms, not Mainland China equivalents:
- 軟體 (not 軟件), 硬體 (not 硬件), 網路 (not 網絡)
- 資訊 (not 信息), 預設 (not 默認), 列印 (not 打印)
- 品質 (not 質量 for "quality"), 影片 (not 視頻)
- 螢幕 (not 屏幕), 程式 (not 程序 for "program")
- 滑鼠 (not 鼠標), 介面 (not 接口 for "interface")
- 伺服器 (not 服務器), 記憶體 (not 內存)

## Punctuation
- Use full-width punctuation in CJK prose: ，。：；！？（）
- Use 「」 for primary quotation marks, 『』 for nested quotes
- Use 、(dunhao) for enumerating items, not ，
- Use 《》 for book/publication titles

## Character Forms
- Use MoE standard forms: 裡 (not 裏), 線 (not 綫), 麵 (not 麪), 著 (not 着 as particle)

## MCP Server
The zhtw-mcp server provides automated zh-TW linting and fixing.
Use `zhtw` with `fix_mode: "lexical_safe"` and `max_errors: 0` as a quality gate before committing Chinese text."#
        .to_string();

    let vscode_settings = r#"{
  "github.copilot.chat.codeGeneration.instructions": [
    {
      "file": ".github/copilot-instructions.md"
    }
  ],
  "mcp": {
    "servers": {
      "zhtw-mcp": {
        "command": "zhtw-mcp",
        "args": [],
        "env": {}
      }
    }
  }
}"#
    .to_string();

    (instructions, vscode_settings)
}

/// Supported host editors for integration setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    ClaudeCode,
    OpenCode,
    Copilot,
    Cursor,
    Windsurf,
    Cline,
    ContinueDev,
    Generic,
}

impl Host {
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "claude_code" | "claude-code" => Some(Self::ClaudeCode),
            "opencode" | "open-code" => Some(Self::OpenCode),
            "copilot" | "github-copilot" => Some(Self::Copilot),
            "cursor" => Some(Self::Cursor),
            "windsurf" => Some(Self::Windsurf),
            "cline" => Some(Self::Cline),
            "continue" | "continue-dev" | "continue.dev" => Some(Self::ContinueDev),
            "generic" => Some(Self::Generic),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::OpenCode => "opencode",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
            Self::Windsurf => "windsurf",
            Self::Cline => "cline",
            Self::ContinueDev => "continue",
            Self::Generic => "generic",
        }
    }
}

/// All supported hosts.
pub const ALL_HOSTS: &[Host] = &[
    Host::ClaudeCode,
    Host::OpenCode,
    Host::Copilot,
    Host::Cursor,
    Host::Windsurf,
    Host::Cline,
    Host::ContinueDev,
    Host::Generic,
];

/// Generate Cursor rules file content.
pub fn cursor_rules() -> String {
    r#"# Cursor Rules: zh-TW Writing Standards (zhtw-mcp)

## Language Standards
All Chinese text in this project must follow Taiwan Ministry of Education (教育部) standards.
The zhtw-mcp MCP server is available for automated enforcement.

## Tool Usage
Use `zhtw` for linting, fixing, and gating zh-TW text:
- Lint: `zhtw({ "text": "...", "content_type": "markdown" })`
- Fix:  `zhtw({ "text": "...", "fix_mode": "lexical_safe", "max_errors": 0 })`
- Gate: `zhtw({ "text": "...", "max_errors": 0 })` — fails if errors > 0

## Key Conventions
- Taiwan terms: 軟體 (not 軟件), 資訊 (not 信息), 預設 (not 默認), 程式 (not 程序)
- Use full-width punctuation in CJK: ，。：；！？
- Quotes: 「」 primary, 『』 nested
- MoE character forms: 裡 (not 裏), 線 (not 綫), 著 (not 着)

## Profiles
- `default`: Standard vocabulary + punctuation
- `strict_moe`: Full MoE enforcement including character variants
- `ui_strings`: Relaxed for software UI (half-width colons allowed)
- `editorial`: AI writing review — detects filler phrases, semantic safety words, copula/passive overuse"#
        .to_string()
}

/// Generate Windsurf rules file content.
pub fn windsurf_rules() -> String {
    r#"# Windsurf Rules: zh-TW Writing Standards

All Chinese text must follow Taiwan MoE (教育部) standards.
The zhtw-mcp MCP server provides automated zh-TW linting and fixing.

## MCP Tool: zhtw
- `zhtw({ "text": "...", "fix_mode": "lexical_safe", "max_errors": 0 })`
- Profiles: default, strict_moe, ui_strings, editorial (AI writing review)
- Content types: plain, markdown

## Taiwan-Standard Terms
軟體 (not 軟件), 資訊 (not 信息), 預設 (not 默認), 程式 (not 程序),
網路 (not 網絡), 硬體 (not 硬件), 品質 (not 質量), 螢幕 (not 屏幕)

## Punctuation
Full-width in CJK prose: ，。：；！？（）
Quotes: 「」 primary, 『』 nested, 《》 book titles
Ellipsis: …… (two U+2026), Em dash: —— (two U+2014)"#
        .to_string()
}

/// Generate Cline rules file content.
pub fn cline_rules() -> String {
    r#"# Cline Rules: zh-TW Writing Standards

## MCP Server
zhtw-mcp provides `zhtw` for Traditional Chinese (Taiwan) text enforcement.

## Workflow
1. When generating Chinese text, use Taiwan-standard vocabulary
2. Before finalizing, run: `zhtw({ "text": "...", "fix_mode": "lexical_safe", "max_errors": 0 })`
3. If `accepted: false`, fix remaining issues and re-check

## Quick Reference
- Terms: 軟體/資訊/預設/程式/網路/硬體/品質/螢幕 (TW standard)
- Punctuation: ，。：；！？ (full-width in CJK), 「」『』 (quotes)
- Profiles: default | strict_moe | ui_strings | editorial (AI writing review)"#
        .to_string()
}

/// Generate Continue.dev MCP configuration.
pub fn continuedev_config() -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": [{
            "name": "zhtw-mcp",
            "command": "zhtw-mcp",
            "args": [],
            "env": {}
        }],
        "customInstructions": "When writing Traditional Chinese (Taiwan) text, use Taiwan MoE standards. Use the zhtw MCP tool to lint and fix text. Key terms: 軟體 (not 軟件), 資訊 (not 信息), 預設 (not 默認). Use full-width punctuation in CJK prose."
    }))
    .unwrap()
}

/// Generate a generic platform-agnostic instruction file.
pub fn generic_instructions() -> String {
    r#"# zhtw-mcp: zh-TW Text Quality Enforcement

## What It Does
zhtw-mcp is an MCP server that enforces Traditional Chinese (Taiwan) writing standards
per the Ministry of Education (教育部) guidelines. It detects mainland Chinese vocabulary,
incorrect punctuation, and non-standard character variants in your text.

## MCP Tool: zhtw
The single unified tool for linting, fixing, and gating zh-TW text.

### Parameters
| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| text | string | (required) | Text to check |
| fix_mode | string | "none" | "none", "orthographic", "lexical_safe", or "lexical_contextual" |
| max_errors | integer | (none) | Gate: reject if errors exceed this |
| profile | string | "default" | "default", "strict_moe", "ui_strings", "editorial" |
| content_type | string | "plain" | "plain" or "markdown" |
| political_stance | string | "roc_centric" | "roc_centric", "international", "neutral" |
| ignore_terms | array | [] | Terms to downgrade to Info severity |
| explain | boolean | false | Attach cultural explanations to issues |

### Workflow
1. Lint: `zhtw({ "text": "...", "content_type": "markdown" })`
2. Fix:  `zhtw({ "text": "...", "fix_mode": "lexical_safe" })`
3. Gate: `zhtw({ "text": "...", "max_errors": 0 })` — accepted=false if errors>0

### MCP Resources
- `zh-tw://style-guide/moe` — Full MoE style guide (punctuation, variants, vocabulary)
- `zh-tw://dictionary/ambiguous` — Terms needing LLM disambiguation

### MCP Prompts
- `normalize_tone` — Editorial persona for naturalizing zh-TW text

## Taiwan-Standard Vocabulary (Common Substitutions)
| Mainland (CN) | Taiwan (TW) | English |
|---------------|-------------|---------|
| 軟件 | 軟體 | Software |
| 信息 | 資訊 | Information |
| 默認 | 預設 | Default |
| 程序 | 程式 | Program |
| 網絡 | 網路 | Network |
| 質量 | 品質 | Quality |

## Punctuation Rules
- Use full-width punctuation in CJK prose: ，。：；！？（）
- Quotes: 「primary」, 『nested』, 《book title》
- Ellipsis: …… (two U+2026), Em dash: —— (two U+2014)
- Enum comma: 、(dunhao) for list items"#
        .to_string()
}

/// Generate integration content for a specific host.
///
/// Returns a JSON-serializable structure with the configuration content.
pub fn generate_for_host(host: Host) -> serde_json::Value {
    match host {
        Host::ClaudeCode => {
            serde_json::json!({
                "host": "claude_code",
                "file": ".claude/CLAUDE.md",
                "instruction": "Append the following section to your project's CLAUDE.md file:",
                "content": claude_code_section(),
            })
        }
        Host::OpenCode => {
            serde_json::json!({
                "host": "opencode",
                "file": ".opencode/skills/zhtw-lint.yaml",
                "instruction": "Save the following as a skill definition file:",
                "content": opencode_skill(),
            })
        }
        Host::Copilot => {
            let (instructions, vscode_settings) = copilot_config();
            serde_json::json!({
                "host": "copilot",
                "files": [
                    {
                        "path": ".github/copilot-instructions.md",
                        "content": instructions,
                    },
                    {
                        "path": ".vscode/settings.json (merge into existing)",
                        "content": vscode_settings,
                    }
                ],
                "instruction": "Create the copilot-instructions.md file and merge the MCP server settings into your VS Code settings.json:",
            })
        }
        Host::Cursor => {
            serde_json::json!({
                "host": "cursor",
                "file": ".cursor/rules",
                "instruction": "Save the following as your Cursor rules file:",
                "content": cursor_rules(),
            })
        }
        Host::Windsurf => {
            serde_json::json!({
                "host": "windsurf",
                "file": ".windsurfrules",
                "instruction": "Save the following as your Windsurf rules file:",
                "content": windsurf_rules(),
            })
        }
        Host::Cline => {
            serde_json::json!({
                "host": "cline",
                "file": ".clinerules",
                "instruction": "Save the following as your Cline rules file:",
                "content": cline_rules(),
            })
        }
        Host::ContinueDev => {
            serde_json::json!({
                "host": "continue",
                "file": ".continue/config.json (merge into existing)",
                "instruction": "Merge the following MCP server configuration into your Continue.dev config:",
                "content": continuedev_config(),
            })
        }
        Host::Generic => {
            serde_json::json!({
                "host": "generic",
                "file": ".zhtw-mcp.md",
                "instruction": "Save the following as a platform-agnostic instruction file that any MCP-aware agent can read:",
                "content": generic_instructions(),
            })
        }
    }
}

/// Generate a zh-TW translation style guide as a JSON setup object.
///
/// Returns a `serde_json::Value` following the same JSON contract as
/// `generate_for_host()`: {host, file, instruction, content}.
/// Designed to be injected into LLM system prompts to prevent common
/// AI writing artifacts at generation time. Covers: cross-strait
/// terminology, semantic safety alternatives, nominalization avoidance,
/// filler prohibition, and verb-driven syntax.
pub fn generate_translation_guide() -> serde_json::Value {
    let guide = translation_guide_text();
    serde_json::json!({
        "host": "translation-guide",
        "file": "(system prompt injection)",
        "instruction": "Inject the following into your LLM system prompt:",
        "content": guide,
    })
}

/// The raw translation guide text content.
fn translation_guide_text() -> String {
    r#"# 繁體中文（台灣）翻譯風格指南

## 目的
本指南用於 LLM 系統提示，確保產出的繁體中文文本符合台灣教育部標準，
避免常見的 AI 翻譯偽跡（translation artifact）。

## 詞彙規範

### 跨海峽術語
使用台灣慣用譯名，避免中國大陸用語：
- 軟體（非「軟件」）、硬體（非「硬件」）
- 程式（非「程序」，指 program）、程式碼（非「代碼」）
- 記憶體（非「內存」）、網路（非「網絡」）
- 資料庫（非「數據庫」）、資料（非「數據」，指 data）
- 伺服器（非「服務器」）、瀏覽器（非簡體「浏览器」）
- 滑鼠（非「鼠標」）、列印（非「打印」）
- 預設（非「默認」）、支援（非「支持」，指 support）

### 語意安全詞
避免直接翻譯「means」為「意味著」。根據語境選擇：
- 定義語境 →「表示」（X 表示 Y）
- 因果語境 →「代表」（這代表我們需要……）
- 解釋語境 →「也就是說」

### 避免繁複動詞
- 避免「作為」「標誌著」「充當」等書面語堆疊，改以簡潔句式表達
- 避免「擁有」「設有」等冗餘動詞（技術文件語境），直接敘述
- 用主動語態取代「被廣泛使用」→「廣泛使用」

## AI 寫作偽跡禁忌

### 禁用填充詞
以下片語在 AI 生成文本中出現頻率異常高，應避免或替換：
- ❌ 值得注意的是 → ✅ 直接陳述事實
- ❌ 需要注意的是 → ✅ 直接陳述事實
- ❌ 更重要的是 → ✅ 精簡為直述句
- ❌ 在某種程度上 → ✅ 刪除或改為具體程度
- ❌ 不容忽視 → ✅ 用具體影響取代
- ❌ 深刻影響 → ✅ 說明具體影響

### 禁用說教句式
- ❌ 讓我們……（祈使句開頭）
- ❌ 我們需要理解……（居高臨下）
- ❌ ……是非常重要的（空泛強調）

### 結構限制
- 避免過度使用列表：列表段落不應超過全文 40%
- 避免公式化段落結尾：「這……證明了」「正是這……讓」
- 避免二元對比堆疊：同一段落不要連續使用「雖然……但」「不僅……更」
- 段落內破折號（——）不超過 2 個
- 避免公式化標題：「挑戰與未來展望」「結論與展望」「核心優勢」

## 標點符號
- 使用全形標點：，。！？；：（）「」『』
- 引號層級：外層「」，內層『』
- 刪節號：使用「……」（兩個 U+2026），非「...」
- 數字範圍：使用「～」或「–」，非「-」

## 語法
- CJK 與半形字母/數字間加一個半形空格
- 避免 的/地/得 誤用
- 句子以動詞驅動，減少名詞化（nominalization）
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_section_contains_tools() {
        let section = claude_code_section();
        assert!(section.contains("zhtw"));
        assert!(!section.contains("zh_lint"));
        assert!(!section.contains("zh_finalize"));
        assert!(!section.contains("zh_apply_fixes"));
    }

    #[test]
    fn claude_code_section_contains_conventions() {
        let section = claude_code_section();
        assert!(section.contains("軟體"));
        assert!(section.contains("資訊"));
        assert!(section.contains("full-width"));
    }

    #[test]
    fn opencode_skill_is_valid_yaml_structure() {
        let skill = opencode_skill();
        assert!(skill.contains("name: zhtw-lint"));
        assert!(skill.contains("zhtw"));
        assert!(!skill.contains("zh_lint"));
        assert!(!skill.contains("zh_finalize"));
        assert!(skill.contains("normalize_tone"));
    }

    #[test]
    fn copilot_config_has_instructions_and_settings() {
        let (instructions, settings) = copilot_config();
        assert!(instructions.contains("軟體"));
        assert!(instructions.contains("full-width"));
        assert!(settings.contains("zhtw-mcp"));
        assert!(settings.contains("mcp"));
    }

    #[test]
    fn host_from_str_parses_all_variants() {
        assert_eq!(Host::from_name("claude_code"), Some(Host::ClaudeCode));
        assert_eq!(Host::from_name("claude-code"), Some(Host::ClaudeCode));
        assert_eq!(Host::from_name("opencode"), Some(Host::OpenCode));
        assert_eq!(Host::from_name("copilot"), Some(Host::Copilot));
        assert_eq!(Host::from_name("github-copilot"), Some(Host::Copilot));
        assert_eq!(Host::from_name("cursor"), Some(Host::Cursor));
        assert_eq!(Host::from_name("windsurf"), Some(Host::Windsurf));
        assert_eq!(Host::from_name("cline"), Some(Host::Cline));
        assert_eq!(Host::from_name("continue"), Some(Host::ContinueDev));
        assert_eq!(Host::from_name("continue-dev"), Some(Host::ContinueDev));
        assert_eq!(Host::from_name("continue.dev"), Some(Host::ContinueDev));
        assert_eq!(Host::from_name("generic"), Some(Host::Generic));
        assert!(Host::from_name("unknown").is_none());
    }

    #[test]
    fn cursor_rules_contains_tool_and_conventions() {
        let rules = cursor_rules();
        assert!(rules.contains("zhtw"));
        assert!(rules.contains("軟體"));
        assert!(rules.contains("full-width"));
    }

    #[test]
    fn windsurf_rules_contains_tool_and_terms() {
        let rules = windsurf_rules();
        assert!(rules.contains("zhtw"));
        assert!(rules.contains("軟體"));
    }

    #[test]
    fn cline_rules_contains_tool() {
        let rules = cline_rules();
        assert!(rules.contains("zhtw"));
    }

    #[test]
    fn continuedev_config_has_mcp_server() {
        let config = continuedev_config();
        assert!(config.contains("zhtw-mcp"));
        assert!(config.contains("mcpServers"));
    }

    #[test]
    fn generic_instructions_comprehensive() {
        let instructions = generic_instructions();
        assert!(instructions.contains("zhtw"));
        assert!(instructions.contains("fix_mode"));
        assert!(instructions.contains("max_errors"));
        assert!(instructions.contains("軟體"));
        assert!(instructions.contains("full-width"));
    }

    #[test]
    fn translation_guide_json_contract() {
        let guide = generate_translation_guide();
        assert!(guide.is_object());
        assert_eq!(guide["host"], "translation-guide");
        assert_eq!(guide["file"], "(system prompt injection)");
        let content = guide["content"].as_str().unwrap();
        assert!(content.contains("繁體中文"));
        assert!(content.contains("值得注意的是"));
        assert!(content.len() > 500);
    }

    #[test]
    fn generate_for_all_hosts_succeeds() {
        for host in ALL_HOSTS {
            let output = generate_for_host(*host);
            assert!(output.is_object());
        }
    }
}
