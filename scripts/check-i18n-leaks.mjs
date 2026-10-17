// i18n 扫描（随 pnpm check 运行），四条防线：
//   1) 已迁移的代码里不得有硬编码中文 —— 必须走 t()；
//   2) 英文资源里不得留中文 —— 键对齐测试只保证「两边都有这个键」，不保证值真的翻了；
//   3) 两种豁免都必须可追溯：整文件 WHITELIST 与英文白名单指向不存在的条目时报错，防止腐烂。
//   4) 工作区修改的中英文资源必须成对出现，避免只改一侧语言。
// 两级豁免：
//   - 整文件 WHITELIST —— 尚未迁移的文件，写明分期，迁移一个摘一个；
//   - 行内 `i18n-exempt: <理由>` —— 不能翻的单行（配置值、正则片段等），必须写清为什么。
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const SRC_ROOT = join(ROOT, "src");
/** 译文所在目录，其内容本身就是中文，不参与代码扫描。 */
const LOCALES_PREFIX = "i18n/locales";
/** 行内豁免标记，写在需要豁免的那一行上（行尾注释即可）。 */
const EXEMPT_MARK = "i18n-exempt";

/** 尚未迁移的文件，写明理由，迁移一个摘一个。 */
const WHITELIST = [
  { file: "api/web-mock.ts", reason: "浏览器调试桩，不随应用发行" },
];

/** 英文资源里允许保留中文的条目。 */
const EN_ALLOWLIST = [
  { file: "i18n/locales/en-US/settings.ts", key: "zh", reason: "语言名按惯例用母语显示（简体中文）" },
];

const CJK = /[一-鿿]/;

function changedFiles() {
  try {
    const output = execFileSync("git", ["status", "--porcelain=v1", "--untracked-files=all"], {
      cwd: ROOT,
      encoding: "utf8",
    });
    return new Set(output.split(/\r?\n/).filter(Boolean).map((line) => {
      const path = line.slice(3);
      const renameSeparator = path.lastIndexOf(" -> ");
      return renameSeparator >= 0 ? path.slice(renameSeparator + 4) : path;
    }));
  } catch {
    return new Set();
  }
}

/**
 * 去掉注释后只剩会被渲染或参与逻辑的代码。注释永远不可能显示给用户，必须剥离，
 * 否则行尾注释会被误报成「未走 t() 的中文」。
 * 块注释替换为等量空白并保留换行，保证行号与原文一致（行内豁免依赖行号对齐）。
 * 行尾注释要求 `//` 前有空白，避免把字符串里的 `https://` 当成注释起点。
 */
const stripComments = (source) =>
  source
    .replace(/\/\*[\s\S]*?\*\//g, (block) => block.replace(/[^\n]/g, " "))
    .replace(/^[ \t]*\/\/.*$/gm, "")
    .replace(/[ \t]\/\/.*$/gm, "");

function walk(dir) {
  return readdirSync(dir, { recursive: true })
    .map((name) => join(dir, name))
    .filter((full) => !statSync(full).isDirectory() && /\.(tsx|ts)$/.test(full) && !/\.test\.(ts|tsx)$/.test(full));
}

const rel = (filePath) => filePath.slice(SRC_ROOT.length + 1).replace(/\\/g, "/");

const allowed = new Set(WHITELIST.map((item) => item.file));
const violations = [];
const files = walk(SRC_ROOT);

for (const filePath of files) {
  const relative = rel(filePath);
  if (relative.startsWith(LOCALES_PREFIX) || allowed.has(relative)) continue;
  const raw = readFileSync(filePath, "utf8");
  const exemptLines = new Set();
  raw.split("\n").forEach((line, index) => {
    if (line.includes(EXEMPT_MARK)) exemptLines.add(index);
  });
  stripComments(raw).split("\n").forEach((line, index) => {
    if (exemptLines.has(index) || !CJK.test(line)) return;
    violations.push({ file: relative, detail: line.trim().slice(0, 80) });
  });
}

/* 英文资源不得留中文：漏翻的键能通过键对齐测试，只有值本身能暴露。 */
const usedAllowlist = new Set();
for (const filePath of walk(join(SRC_ROOT, "i18n", "locales", "en-US"))) {
  const relative = rel(filePath);
  for (const line of stripComments(readFileSync(filePath, "utf8")).split("\n")) {
    const match = line.match(/^\s*([A-Za-z0-9_]+):\s*"(.*?)",?\s*$/);
    if (!match) continue;
    const [, key, value] = match;
    if (!CJK.test(value)) continue;
    if (EN_ALLOWLIST.some((item) => item.file === relative && item.key === key)) {
      usedAllowlist.add(`${relative}::${key}`);
      continue;
    }
    violations.push({ file: relative, detail: `${key}: "${value}"  ← 英文资源里出现中文（漏翻？）` });
  }
}

const staleWhitelist = WHITELIST.filter((item) => !files.some((f) => rel(f) === item.file));
const staleEnAllowlist = EN_ALLOWLIST.filter((item) => !usedAllowlist.has(`${item.file}::${item.key}`));

const changed = changedFiles();
for (const file of changed) {
  const match = file.match(/^src\/i18n\/locales\/(zh-CN|en-US)\/(.+)$/);
  if (!match) continue;
  const [, locale, relativePath] = match;
  const counterpartLocale = locale === "zh-CN" ? "en-US" : "zh-CN";
  const counterpart = `src/i18n/locales/${counterpartLocale}/${relativePath}`;
  if (existsSync(join(ROOT, counterpart)) && !changed.has(counterpart)) {
    violations.push({ file, detail: `对应的 ${counterpart} 未同步修改` });
  }
}

if (violations.length) {
  console.error(`✖ i18n 扫描发现 ${violations.length} 处问题：`);
  for (const v of violations) console.error(`  ${v.file}: ${v.detail}`);
  console.error("处理方式：代码里的中文改用 t()（或整文件进 WHITELIST、单行加 `// i18n-exempt: 理由`）；英文资源里的中文补上译文。");
  process.exit(1);
}
if (staleWhitelist.length) {
  console.error("✖ WHITELIST 指向不存在的文件（已过期，请清理）：");
  for (const item of staleWhitelist) console.error(`  ${item.file}  ← ${item.reason}`);
  process.exit(1);
}
if (staleEnAllowlist.length) {
  console.error("✖ 英文白名单已过期（对应条目不再是中文了，请删除）：");
  for (const item of staleEnAllowlist) console.error(`  ${item.file} 的 ${item.key}  ← ${item.reason}`);
  process.exit(1);
}
console.log("✔ i18n 扫描通过（无硬编码中文；英文资源无漏翻）");
