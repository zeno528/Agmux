---
name: ci-check
description: 在本地完整复刻 GitHub Actions CI 的检查链（tsc 类型检查、Vitest、cargo fmt、clippy -D warnings、cargo test、vite 生产构建），失败就地修复并复跑直到全绿，确保推送后 CI 一次通过、不用返工。当用户说"检查一下"、"跑一下 CI"、"本地 CI"、"推送前检查"、"check 一下"、"preflight" 时使用。
---

# 本地 CI 预检

CI 的 check job（`.github/workflows/ci.yml`）跑 `pnpm install --frozen-lockfile` → `pnpm check` → `pnpm build`，一轮至少十几分钟，推上去红了再返工很浪费。本 skill 在本地跑完全等价的链条，红了就地修，全绿了才推。

与 CI 的对应关系：

| CI 步骤 | 本地做法 |
|---|---|
| `pnpm install --frozen-lockfile` | package.json / pnpm-lock.yaml 有变动时先本地 `pnpm install` 并提交 lockfile |
| `pnpm check`（typecheck + fmt + clippy + test） | 同一条命令，完全一致 |
| `pnpm build`（vite 生产构建） | 同一条命令，完全一致 |
| macOS job 的 `cargo test` | Windows 本地无法覆盖；已知缺口，仅平台特定问题才会踩到 |

## Instructions

### Step 1: 跑完整链条

```
pnpm check && pnpm build
```

后台执行（约 3-5 分钟），完成前不要下结论。

### Step 2: 失败就地修

按失败阶段处理，修的是根因，不是绕过检查：

- **tsc/Vitest 报错**：修类型或测试错误本身；禁止改 tsconfig / 加 any 绕过。
- **cargo fmt --check**：直接执行 `cargo fmt --all --manifest-path src-tauri/Cargo.toml`，然后展示改了哪些文件。
- **clippy -D warnings**：逐条修到零警告；确需 `#[allow]` 时必须说明理由并留在代码注释里。
- **cargo test 失败**：先判断是本次改动引入还是环境差异，修根因；与改动无关的既有失败要报告而不是静默跳过。
- **vite build 失败**：通常是上游类型/导入问题，按报错定位。

修完回到 Step 1 重跑整条链（不要只跑失败的那一段），直到全绿。

### Step 3: 汇报

用表格逐项对齐 CI 步骤汇报结果，最后明确一句话结论："本地全绿，推送后 CI 预期通过"，并注明 macOS cargo test 未本地覆盖。

## 示例

**场景**：用户说"跑一下 CI 再推"

1. 后台执行 `pnpm check && pnpm build`
2. `cargo fmt --check` 红 → 执行 `cargo fmt`，修了 commands.rs / lib.rs / services.rs
3. 复跑整链：typecheck ✅ fmt ✅ clippy ✅ test 88 passed ✅ build ✅
4. 汇报表格 + "本地全绿，可推送"
5. 用户确认后再执行推送

## Troubleshooting

**本地过、CI 红（或反过来的 fmt 差异）**：工具链不一致。确认 `rustc --version` 与仓库根目录 `rust-toolchain.toml` 固定的工具链一致；不一致时先检查 Rustup 是否正确读取项目配置。

**CI 的 `pnpm install --frozen-lockfile` 失败但本地正常**：lockfile 与 package.json 不同步。本地 `pnpm install` 重新生成并提交 pnpm-lock.yaml。

**用户要求推送但本轮改动还没跑过本链条**：先建议跑一遍再推；用户坚持直接推则照做，但说明风险。
