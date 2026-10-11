//! 核心后端热路径基准：settings 读写、config.toml 解析/应用、SQLite 配置查询。
//!
//! 运行：cargo bench --bench config_bench --manifest-path src-tauri/Cargo.toml
//! CI 解析需要 bencher 输出：加 `-- --output-format bencher`（github-action-benchmark 的 cargo 解析器只认该格式）
//! 全部使用临时目录（tempfile），不触碰真实 ~/.agofra 与 ~/.codex 数据。

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};

use agofra_lib::codex::config::{
    apply_to_document, capture_from_document, format_document, parse_document, validate_document,
};
use agofra_lib::database::Database;
use agofra_lib::models::{CodexProfilePayload, Settings};
use agofra_lib::paths::from_home;
use agofra_lib::services::AppContext;

/// 与真实 ~/.codex/config.toml 结构一致的最小样本（含注释与无关配置，验证保真逻辑）。
const SAMPLE_CONFIG_TOML: &str = r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"
model_catalog_json = "zai.json"

[features]
goals = true

[mcp_servers.test]
command = "node"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.z.ai"
wire_api = "responses"
experimental_bearer_token = "secret"

[model_providers.Old]
name = "Old"
"#;

fn bench_settings_read(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let context = AppContext::new(from_home(dir.path()).unwrap()).unwrap();
    context.save_settings(&Settings::default()).unwrap();

    c.bench_function("settings_read", |b| {
        b.iter(|| black_box(context.settings().unwrap()));
    });
}

fn bench_settings_write(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let context = AppContext::new(from_home(dir.path()).unwrap()).unwrap();
    let settings = Settings::default();

    c.bench_function("settings_write", |b| {
        b.iter(|| black_box(context.save_settings(black_box(&settings)).unwrap()));
    });
}

fn bench_config_toml_parse_capture(c: &mut Criterion) {
    c.bench_function("config_toml_parse_capture", |b| {
        b.iter(|| {
            let document = parse_document(black_box(SAMPLE_CONFIG_TOML)).unwrap();
            black_box(capture_from_document(&document).unwrap());
        })
    });
}

fn bench_config_toml_apply(c: &mut Criterion) {
    let payload = capture_from_document(&parse_document(SAMPLE_CONFIG_TOML).unwrap()).unwrap();

    c.bench_function("config_toml_apply", |b| {
        b.iter(|| {
            let mut document = parse_document(black_box(SAMPLE_CONFIG_TOML)).unwrap();
            apply_to_document(&mut document, black_box(&payload)).unwrap();
            black_box(document.to_string());
        })
    });
}

/// 编辑器 linter 热路径：每次防抖后触发一次，分别测有效文档（最常见）与含错文档（错误恢复）。
fn bench_config_toml_validate_format(c: &mut Criterion) {
    let broken = SAMPLE_CONFIG_TOML.replace("wire_api = \"responses\"", "wire_api =");

    c.bench_function("config_toml_validate_valid", |b| {
        b.iter(|| black_box(validate_document(black_box(SAMPLE_CONFIG_TOML)).len()))
    });
    c.bench_function("config_toml_validate_broken", |b| {
        b.iter(|| black_box(validate_document(black_box(&broken)).len()))
    });
    c.bench_function("config_toml_format", |b| {
        b.iter(|| black_box(format_document(black_box(SAMPLE_CONFIG_TOML))))
    });
}

fn bench_db_profiles_query(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let paths = from_home(dir.path()).unwrap();
    let db = Database::open(&paths).unwrap();
    for index in 0..20 {
        db.codex_insert_profile(
            &format!("供应商 {index}"),
            &CodexProfilePayload::default(),
            &index.to_string(),
        )
        .unwrap();
    }

    c.bench_function("db_profiles_query_20", |b| {
        b.iter(|| black_box(db.codex_profiles().unwrap().len()));
    });
}

criterion_group!(
    benches,
    bench_settings_read,
    bench_settings_write,
    bench_config_toml_parse_capture,
    bench_config_toml_apply,
    bench_config_toml_validate_format,
    bench_db_profiles_query,
);
criterion_main!(benches);
