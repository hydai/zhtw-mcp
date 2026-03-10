// Criterion benchmarks for zhtw-mcp scanning pipeline.
//
// Covers the benchmark targets:
//   1. Scanner construction (Aho-Corasick automaton build)
//   2. scan() on 1KB / 10KB / 100KB mixed CJK+ASCII text
//   3. scan_profiled() with StrictMoe profile
//   4. apply_fixes_with_context() with 50 concurrent fixes
//   5. Markdown exclusion pass (build_markdown_excluded_ranges)
//   6. FMM segmenter on 100-char text

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use zhtw_mcp::engine::markdown::build_markdown_excluded_ranges;
use zhtw_mcp::engine::scan::Scanner;
use zhtw_mcp::engine::segment::Segmenter;
use zhtw_mcp::fixer::{apply_fixes_with_context, FixMode};
use zhtw_mcp::rules::loader::load_embedded_ruleset;
use zhtw_mcp::rules::ruleset::Profile;

// Test data generation

/// Base paragraph (~200 bytes) mixing CJK prose with Mainland terms the
/// scanner will flag, plus some ASCII for realism.  Repeating this block
/// scales linearly while keeping a consistent hit ratio.
const BASE_PARAGRAPH: &str = "\
台灣的軟件工程師使用人工智能技術開發應用程序。\
他們的網絡質量很高，信息安全也很重要。\
The server handles HTTP requests via async runtime.\n\
數據庫中存儲了用戶的個人信息和視頻文件。\
項目採用敏捷開發的方法論，並行運算能力很強。\n";

/// Build mixed CJK+ASCII text of approximately `target_bytes` size.
fn generate_text(target_bytes: usize) -> String {
    let repeats = (target_bytes / BASE_PARAGRAPH.len()).max(1);
    let mut text = BASE_PARAGRAPH.repeat(repeats);
    if text.len() > target_bytes {
        text.truncate(text.floor_char_boundary(target_bytes));
    }
    text
}

/// Generate Markdown text with code blocks, inline code, frontmatter,
/// and prose for the exclusion-pass benchmark.
fn generate_markdown(target_bytes: usize) -> String {
    let block = "\
---\ntitle: 測試文件\ndate: 2024-01-01\n---\n\n\
# 標題：軟件開發指南\n\n\
台灣的軟件工程師使用 `println!` 來調試程序。\n\n\
```rust\nfn main() {\n    let x = 軟件;\n    println!(\"{x}\");\n}\n```\n\n\
數據庫中存儲了用戶的信息。The server runs on port 8080.\n\n\
> 引用文字：這是一段 `inline code` 和一些文字。\n\n\
- 項目一：網絡質量\n  - `async` 子項目\n- 項目二：信息安全\n\n";

    let repeats = (target_bytes / block.len()).max(1);
    let mut text = block.repeat(repeats);
    if text.len() > target_bytes {
        text.truncate(text.floor_char_boundary(target_bytes));
    }
    text
}

/// 100-char CJK string for the FMM segmenter benchmark.
/// Exactly 100 Chinese characters covering a mix of dictionary and
/// non-dictionary terms.
const SEGMENTER_INPUT: &str = "\
台灣的軟體工程師使用人工智慧技術開發應用程式。\
他們的網路品質很高，資訊安全也很重要。\
資料庫中儲存了使用者的個人資訊和影片檔案。\
這個專案採用敏捷開發的方法論，並行運算能力很強大。\
程式語言的選擇也非常關鍵。";

// 1. Scanner construction

fn bench_scanner_construction(c: &mut Criterion) {
    let ruleset = load_embedded_ruleset().expect("load embedded ruleset");

    c.bench_function("scanner_construction", |b| {
        b.iter(|| {
            let scanner = Scanner::new(
                black_box(ruleset.spelling_rules.clone()),
                black_box(ruleset.case_rules.clone()),
            );
            black_box(&scanner);
        });
    });
}

// 1b. Construction breakdown: Segmenter vs Aho-Corasick builds

fn bench_construction_breakdown(c: &mut Criterion) {
    use aho_corasick::{AhoCorasickBuilder, MatchKind};

    let ruleset = load_embedded_ruleset().expect("load embedded ruleset");
    let spelling_rules: Vec<_> = ruleset
        .spelling_rules
        .iter()
        .filter(|r| !r.disabled)
        .cloned()
        .collect();
    let case_rules: Vec<_> = ruleset
        .case_rules
        .iter()
        .filter(|r| !r.disabled)
        .cloned()
        .collect();

    let mut group = c.benchmark_group("construction_breakdown");

    group.bench_function("segmenter_from_rules", |b| {
        b.iter(|| {
            let seg = Segmenter::from_rules(black_box(&spelling_rules));
            black_box(&seg);
        });
    });

    group.bench_function("spelling_aho_corasick", |b| {
        let patterns: Vec<&str> = spelling_rules.iter().map(|r| r.from.as_str()).collect();
        b.iter(|| {
            let ac = AhoCorasickBuilder::new()
                .match_kind(MatchKind::LeftmostLongest)
                .build(black_box(&patterns))
                .expect("build spelling AC");
            black_box(&ac);
        });
    });

    group.bench_function("case_aho_corasick", |b| {
        let patterns: Vec<String> = case_rules.iter().map(|r| r.term.to_lowercase()).collect();
        b.iter(|| {
            let ac = AhoCorasickBuilder::new()
                .match_kind(MatchKind::LeftmostLongest)
                .ascii_case_insensitive(true)
                .build(black_box(&patterns))
                .expect("build case AC");
            black_box(&ac);
        });
    });

    group.finish();
}

// 2. scan() on 1KB / 10KB / 100KB

fn bench_scan(c: &mut Criterion) {
    let ruleset = load_embedded_ruleset().expect("load embedded ruleset");
    let scanner = Scanner::new(ruleset.spelling_rules, ruleset.case_rules);

    let sizes: &[(usize, &str)] = &[(1_024, "1KB"), (10_240, "10KB"), (102_400, "100KB")];

    let mut group = c.benchmark_group("scan");
    for &(size, label) in sizes {
        let text = generate_text(size);
        group.bench_with_input(BenchmarkId::from_parameter(label), &text, |b, text| {
            b.iter(|| {
                let issues = scanner.scan(black_box(text));
                black_box(&issues);
            });
        });
    }
    group.finish();
}

// 3. scan_profiled() with StrictMoe

fn bench_scan_profiled_strict_moe(c: &mut Criterion) {
    let ruleset = load_embedded_ruleset().expect("load embedded ruleset");
    let scanner = Scanner::new(ruleset.spelling_rules, ruleset.case_rules);

    let sizes: &[(usize, &str)] = &[(1_024, "1KB"), (10_240, "10KB"), (102_400, "100KB")];

    let mut group = c.benchmark_group("scan_profiled_strict_moe");
    for &(size, label) in sizes {
        let text = generate_text(size);
        group.bench_with_input(BenchmarkId::from_parameter(label), &text, |b, text| {
            b.iter(|| {
                let issues = scanner.scan_profiled(black_box(text), Profile::StrictMoe);
                black_box(&issues);
            });
        });
    }
    group.finish();
}

// 4. apply_fixes_with_context() with 50 concurrent fixes

fn bench_apply_fixes(c: &mut Criterion) {
    let ruleset = load_embedded_ruleset().expect("load embedded ruleset");
    let segmenter = Segmenter::from_rules(&ruleset.spelling_rules);
    let scanner = Scanner::new(ruleset.spelling_rules, ruleset.case_rules);

    // Generate enough text to produce at least 50 issues.
    // The base paragraph has ~8 flaggable terms, so 10KB should yield plenty.
    let text = generate_text(10_240);
    let mut issues = scanner.scan(&text).issues;

    // Cap at exactly 50 issues for a controlled benchmark.
    issues.truncate(50);

    // Ensure we actually have issues to fix (sanity check at setup time).
    assert!(
        !issues.is_empty(),
        "benchmark setup: scanner found no issues in generated text"
    );

    c.bench_function("apply_fixes_50_issues", |b| {
        b.iter(|| {
            let result = apply_fixes_with_context(
                black_box(&text),
                black_box(&issues),
                FixMode::Safe,
                &[],
                Some(&segmenter),
            );
            black_box(&result);
        });
    });
}

// 4b. Context-clue-heavy scan

/// Text with terms that trigger context_clue rules (函數, 實現, 配置, etc.).
/// This exercises the segmentation cache: many AC matches in close proximity
/// require context-clue resolution on overlapping windows.
const CONTEXT_CLUE_PARAGRAPH: &str = "\
在軟件工程中，函數的實現需要考慮配置管理和代碼質量。\
開發人員使用變量和地址來處理數據結構中的信息。\
刷新頁面後，全局的變量可能被實現為回調函數。\
交互式界面的實現涉及事務處理和場景部署。\
運行時環境中的配置需要注意並行計算的實現。\n";

fn bench_scan_context_clues(c: &mut Criterion) {
    let ruleset = load_embedded_ruleset().expect("load embedded ruleset");
    let scanner = Scanner::new(ruleset.spelling_rules, ruleset.case_rules);

    let sizes: &[(usize, &str)] = &[(1_024, "1KB"), (10_240, "10KB"), (102_400, "100KB")];

    let mut group = c.benchmark_group("scan_context_clues");
    for &(size, label) in sizes {
        let repeats = (size / CONTEXT_CLUE_PARAGRAPH.len()).max(1);
        let mut text = CONTEXT_CLUE_PARAGRAPH.repeat(repeats);
        if text.len() > size {
            text.truncate(text.floor_char_boundary(size));
        }
        group.bench_with_input(BenchmarkId::from_parameter(label), &text, |b, text| {
            b.iter(|| {
                let output = scanner.scan(black_box(text));
                black_box(&output);
            });
        });
    }
    group.finish();
}

// 5. Markdown exclusion pass

fn bench_markdown_exclusion(c: &mut Criterion) {
    let sizes: &[(usize, &str)] = &[(1_024, "1KB"), (10_240, "10KB"), (102_400, "100KB")];

    let mut group = c.benchmark_group("markdown_exclusion");
    for &(size, label) in sizes {
        let md = generate_markdown(size);
        group.bench_with_input(BenchmarkId::from_parameter(label), &md, |b, md| {
            b.iter(|| {
                let ranges = build_markdown_excluded_ranges(black_box(md));
                black_box(&ranges);
            });
        });
    }
    group.finish();
}

// 6. FMM segmenter on 100-char text

fn bench_segmenter(c: &mut Criterion) {
    let ruleset = load_embedded_ruleset().expect("load embedded ruleset");
    let segmenter = Segmenter::from_rules(&ruleset.spelling_rules);

    // Verify the input is roughly 100 chars.
    let char_count = SEGMENTER_INPUT.chars().count();
    assert!(
        (90..=110).contains(&char_count),
        "segmenter input should be ~100 chars, got {char_count}"
    );

    c.bench_function("segmenter_100_chars", |b| {
        b.iter(|| {
            let tokens = segmenter.segment(black_box(SEGMENTER_INPUT));
            black_box(&tokens);
        });
    });
}

// Criterion harness

criterion_group!(
    benches,
    bench_scanner_construction,
    bench_construction_breakdown,
    bench_scan,
    bench_scan_profiled_strict_moe,
    bench_apply_fixes,
    bench_scan_context_clues,
    bench_markdown_exclusion,
    bench_segmenter,
);
criterion_main!(benches);
