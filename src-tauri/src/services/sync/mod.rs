//! live→数据库回写的统一时机层。
//! 「何时回写」只有这一份定义，引擎无关；每个客户端注册一个执行体（怎么解析、往哪写、什么算脏）。
//! B 类引擎自有钩子（Codex 的 live auth.json OAuth 回写）是异步路径、管理器由调用方持有，
//! 不走时机枚举，由调用方直连 `sync_live_oauth_auth`；新客户端若有凭证类文件，
//! 必须实现等价的新鲜度判定（归属 + 新旧，对齐 `sync_external_auth_json`）。

use std::sync::OnceLock;

use super::codex_profiles::LiveReadError;
use super::AppContext;
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientId {
    Codex,
    Claude,
}

impl ClientId {
    /// 日志里用的产品名。`{:?}` 打出来的是内部枚举名 `Claude`，
    /// 对着日志排查的人不直观——统一用产品名 `Claude Code` / `Codex`。
    /// 返回值带空格，落日志时按仓库规范用 `{:?}` 编码。
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }
}

/// 回写时机。被动时机（StateRefresh）遍历全部注册执行体；其余时机只作用于 `client` 指定的执行体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncTrigger {
    /// 任何主动刷新聚合状态：启动预发、窗口激活、UI 刷新、托盘切换、页内 refresh。
    /// ponytail: get_state 单入口区分不了这些来源，收敛为一种被动时机。
    StateRefresh,
    /// 覆盖 live 之前（应用/切换供应商）。target_id = 即将写入的目标配置。
    BeforeLiveOverwrite { client: ClientId, target_id: String },
    /// 读取配置详情之前（打开编辑页）。目标就是激活配置时才需要同步。
    BeforeProfileRead { client: ClientId, target_id: String },
    /// 复制配置之前（副本必须取最新快照）。目标就是激活配置时才需要同步。
    BeforeProfileClone { client: ClientId, target_id: String },
    /// 捕获当前 live 为新配置之后。
    AfterCapture { client: ClientId },
    /// 拉起该客户端进程之前（目前仅 Codex 的 restart_codex 构造）。
    BeforeProcessStart { client: ClientId },
}

impl SyncTrigger {
    fn is_passive(&self) -> bool {
        matches!(self, SyncTrigger::StateRefresh)
    }

    fn client(&self) -> ClientId {
        match self {
            SyncTrigger::BeforeLiveOverwrite { client, .. }
            | SyncTrigger::BeforeProfileRead { client, .. }
            | SyncTrigger::BeforeProfileClone { client, .. }
            | SyncTrigger::AfterCapture { client }
            | SyncTrigger::BeforeProcessStart { client } => *client,
            _ => ClientId::Codex,
        }
    }

    /// logfmt 的 trigger 取值（snake_case）。禁用 `{:?}`：枚举 Debug 会把字段
    /// 结构体（含花括号与空格）整块灌进日志值，撑破 logfmt 语法。
    fn token(&self) -> &'static str {
        match self {
            SyncTrigger::StateRefresh => "state_refresh",
            SyncTrigger::BeforeLiveOverwrite { .. } => "before_live_overwrite",
            SyncTrigger::BeforeProfileRead { .. } => "before_profile_read",
            SyncTrigger::BeforeProfileClone { .. } => "before_profile_clone",
            SyncTrigger::AfterCapture { .. } => "after_capture",
            SyncTrigger::BeforeProcessStart { .. } => "before_process_start",
        }
    }

    /// 目标配置 ID（仅主动时机携带）；被动轮询无目标，返回 None 不落该字段。
    fn target_id(&self) -> Option<&str> {
        match self {
            SyncTrigger::BeforeLiveOverwrite { target_id, .. }
            | SyncTrigger::BeforeProfileRead { target_id, .. }
            | SyncTrigger::BeforeProfileClone { target_id, .. } => Some(target_id),
            _ => None,
        }
    }
}

/// 调用方已解析的 live 材料：已持有解析结果时直接复用，避免启动路径二次读盘。
/// 各执行体按需取用自己客户端的字段。
#[derive(Default, Clone, Copy)]
pub struct SyncMaterial<'a> {
    pub codex_document: Option<&'a toml_edit::DocumentMut>,
}

/// 回写结局的种类：把"没写库"拆成具体守卫，排查时不必再靠排除法反推是哪一道拦的。
/// 各情形的语义与返回规约只在 `ClientSync` 的守卫契约表里讲一遍，不在此重复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    Wrote,
    NoActiveProfile,
    TargetMismatch,
    Unchanged,
    LiveAbsent,
    LiveUnreadable,
    LiveParseError,
    WriteFailed,
}

impl SyncKind {
    /// logfmt 的 reason 取值，机器定位靠它、不靠 msg 文案。
    fn reason(&self) -> &'static str {
        match self {
            Self::Wrote => "wrote",
            Self::NoActiveProfile => "no_active",
            Self::TargetMismatch => "target_mismatch",
            Self::Unchanged => "unchanged",
            Self::LiveAbsent => "live_absent",
            Self::LiveUnreadable => "live_unreadable",
            Self::LiveParseError => "live_parse_error",
            Self::WriteFailed => "write_failed",
        }
    }

    /// 失败结局的 failure_kind；None 表示非常态失败（再按 outcome 分成功/跳过）。
    /// "哪些算失败、日志里叫什么"只在这一处枚举，新增失败变体时同步补这里。
    fn failure_kind(&self) -> Option<&'static str> {
        match self {
            Self::LiveUnreadable => Some("io_error"),
            Self::LiveParseError => Some("parse_error"),
            Self::WriteFailed => Some("internal"),
            _ => None,
        }
    }

    /// logfmt 的 outcome 取值：真写了是 success，各类守卫拦下是 skipped，
    /// 带.failure_kind 的是 failure。跳过与成功的分界：数据库有没有被本轮改变。
    fn outcome(&self) -> &'static str {
        match self {
            Self::Wrote => "success",
            Self::NoActiveProfile | Self::TargetMismatch | Self::Unchanged | Self::LiveAbsent => {
                "skipped"
            }
            Self::LiveUnreadable | Self::LiveParseError | Self::WriteFailed => "failure",
        }
    }

    /// 人看的中文结论：主语写全、术语对齐 UI（实时配置/激活配置/数据库），
    /// 机器定位靠 reason，msg 只做结论。守卫语义变更时同步改这里。
    fn message(&self) -> &'static str {
        match self {
            Self::Wrote => "实时配置已回写数据库",
            Self::NoActiveProfile => "当前无激活配置，跳过回写",
            Self::TargetMismatch => "读取对象不是当前激活配置，跳过回写",
            Self::Unchanged => "实时配置与数据库一致",
            Self::LiveAbsent => "实时文件不存在，跳过回写",
            Self::LiveUnreadable => "实时文件读取失败",
            Self::LiveParseError => "实时配置解析失败，保留数据库快照",
            Self::WriteFailed => "数据库写入失败",
        }
    }
}

/// 一次回写的结局：种类 + 涉及的配置名。
/// 配置名取自本轮已经读出来的那一行 profile（构造 input 时已 clone 过），不额外查库，
/// 因此加进日志不影响冷启动预算。无激活/与本轮无关时没有配置可指，`profile` 为 None。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOutcome {
    pub kind: SyncKind,
    pub profile: Option<String>,
}

impl SyncOutcome {
    /// 本轮没落到具体配置：无激活 / 目标非激活 / live 都不存在。
    pub(super) fn bare(kind: SyncKind) -> Self {
        Self {
            kind,
            profile: None,
        }
    }

    /// 本轮确实对着某个配置做的判断，名字一并带出去供日志定位。
    pub(super) fn of(kind: SyncKind, profile: &str) -> Self {
        Self {
            kind,
            profile: Some(profile.to_string()),
        }
    }
}

/// 每客户端执行体。新客户端只需实现这一个 trait。
///
/// ## 新客户端必须遵守的四道守卫
///
/// 回写的安全底线是：**外部损坏的内容绝不能覆盖库内最后一次有效快照**。
/// 实现 `sync` 时必须逐条落到具体 `SyncKind`，不许沉默地返回"没写"：
///
/// | 情形 | 返回 |
/// |---|---|
/// | 没有激活配置（此时**不要读 live**，冷启动预算） | `bare(NoActiveProfile)` |
/// | 时机目标不是激活配置（切到自身 / 读的非激活项） | `bare(TargetMismatch)` |
/// | live 文件不存在（首次运行等正常态） | `of(LiveAbsent, name)` |
/// | live 存在但读不了（权限、被占用） | `of(LiveUnreadable, name)` |
/// | live 解析失败（被外部改坏） | `of(LiveParseError, name)` |
/// | live 与库内一致 | `of(Unchanged, name)` |
/// | 内容确实不同且已写库 | `of(Wrote, name)` |
/// | 写库失败（Codex 侧已由 `record_event` 落事件，这里只补可观测性） | `of(WriteFailed, name)` |
///
/// 前三种 `bare` 之外都必须带出配置名，否则日志里只剩"没回写"，看不出是谁。
///
/// `LiveAbsent` 与 `LiveUnreadable` 必须分开：前者是正常态（不该报警），
/// 后者是故障（该报 Warn）。`read_to_string(...).ok()` 这类写法会把两者
/// 混成一坨，正是当初回写被静默吞掉的原因。
///
/// 判定"一致"的条件由各客户端自己定（Codex 比整个 payload 结构相等，
/// Claude 比 base_url/token/model + 全文），不要求统一；**但读取激活行一律用
/// 主键点查，不要全表拉取再 find**。
///
/// 新增客户端后请照 `claude.rs` 的 `corrupted_live_settings_report_parse_error_and_keep_last_valid_snapshot`
/// 补一个同形状的回归测试：破坏 live → 断言 `LiveParseError` → 断言快照没被覆盖。
pub trait ClientSync: Send + Sync {
    fn id(&self) -> ClientId;

    /// 执行回写。守卫语义见上方契约表：四种跳过必须可辨，失败必须能被日志定位。
    fn sync(
        &self,
        ctx: &AppContext,
        trigger: &SyncTrigger,
        material: SyncMaterial<'_>,
    ) -> AppResult<SyncOutcome>;
}

/// 宽松读路径：调用方已解析的文档优先，否则自行读取 live。
/// 严格读（`live_document_checked`）而非 `.ok()` 吞错误：读不了与解析失败分别落到
/// `LiveUnreadable` / `LiveParseError` 走 Warn，否则 Codex 侧会重演 Claude 这次的"静默不回写"。
fn sync_loose(
    ctx: &AppContext,
    document: Option<&toml_edit::DocumentMut>,
) -> AppResult<SyncOutcome> {
    match document {
        Some(document) => ctx.sync_active_profile_document(document),
        None => match ctx.live_document_checked() {
            Ok(Some(document)) => ctx.sync_active_profile_document(&document),
            Ok(None) => Ok(codex_guarded(SyncKind::LiveAbsent, ctx)),
            Err(LiveReadError::Unreadable) => Ok(codex_guarded(SyncKind::LiveUnreadable, ctx)),
            Err(LiveReadError::Parse) => Ok(codex_guarded(SyncKind::LiveParseError, ctx)),
        },
    }
}

/// Codex 侧 live 缺席/故障的结局统一带出配置名（守卫契约表：除两条 bare 外必须带名）。
/// 主键点查，只在这几条低频路径上花这一次查询，不影响常态与冷启动预算。
fn codex_guarded(kind: SyncKind, ctx: &AppContext) -> SyncOutcome {
    let name = ctx
        .active_profile_state()
        .ok()
        .flatten()
        .and_then(|id| ctx.database.codex_profile(&id).ok())
        .map(|profile| profile.name);
    match name {
        Some(name) => SyncOutcome::of(kind, &name),
        None => SyncOutcome::bare(kind),
    }
}

/// Codex 执行体：行为对齐原 6 个 A 类触发点（守卫语义只搬家不改写）。
struct CodexSyncBody;

impl ClientSync for CodexSyncBody {
    fn id(&self) -> ClientId {
        ClientId::Codex
    }

    fn sync(
        &self,
        ctx: &AppContext,
        trigger: &SyncTrigger,
        material: SyncMaterial<'_>,
    ) -> AppResult<SyncOutcome> {
        match trigger {
            // 覆盖 live 前（原 autosync_active_profile）：无激活 / 自切不做无意义写；
            // 错误向上传播——切换失败优先于覆盖 live。
            SyncTrigger::BeforeLiveOverwrite { target_id, .. } => {
                let Some(active_id) = ctx.active_profile_state()? else {
                    return Ok(SyncOutcome::bare(SyncKind::NoActiveProfile));
                };
                if active_id == *target_id {
                    return Ok(SyncOutcome::bare(SyncKind::TargetMismatch));
                }
                sync_loose(ctx, material.codex_document)
            }
            // 打开编辑页/复制：目标就是激活配置时才同步（错误被调用方忽略）
            SyncTrigger::BeforeProfileRead { target_id, .. }
            | SyncTrigger::BeforeProfileClone { target_id, .. } => {
                if ctx.active_profile_state()?.as_deref() != Some(target_id.as_str()) {
                    return Ok(SyncOutcome::bare(SyncKind::TargetMismatch));
                }
                sync_loose(ctx, material.codex_document)
            }
            // 拉起进程前：严格读 live，读不了/解析失败只记日志不拦——Codex 自己会报错。
            // 这条 Warn 说的是"Codex 起不来"这个产品影响，与 harvest 的回写结局是
            // 两件事，故不合并；且刻意不带 error 正文：toml_edit 的解析报错会内嵌
            // 出错那一行的原文，可能是密钥。
            SyncTrigger::BeforeProcessStart { .. } => match ctx.live_document_checked() {
                Ok(Some(document)) => ctx.sync_active_profile_document(&document),
                Ok(None) => Ok(codex_guarded(SyncKind::LiveAbsent, ctx)),
                Err(LiveReadError::Unreadable) => {
                    tauri_plugin_log::log::warn!(
                        "[app.config.read] outcome=failure failure_kind=io_error msg=\"config.toml 读不了，Codex 可能无法启动\""
                    );
                    Ok(codex_guarded(SyncKind::LiveUnreadable, ctx))
                }
                Err(LiveReadError::Parse) => {
                    tauri_plugin_log::log::warn!(
                        "[app.config.parse] outcome=failure failure_kind=parse_error msg=\"config.toml 无法解析，Codex 可能无法启动\""
                    );
                    Ok(codex_guarded(SyncKind::LiveParseError, ctx))
                }
            },
            // 被动刷新与捕获后：无激活直接跳过且不读 live 文件（冷启动预算）
            SyncTrigger::StateRefresh | SyncTrigger::AfterCapture { .. } => {
                if ctx.active_profile_state()?.is_none() {
                    return Ok(SyncOutcome::bare(SyncKind::NoActiveProfile));
                }
                sync_loose(ctx, material.codex_document)
            }
        }
    }
}

/// Claude 执行体：守卫沿用 sync_active_claude_settings 的四道守卫
/// （无激活/读不了/解析失败/无差异一律不写库，外部损坏绝不覆盖最后有效快照）。
/// 产品事实：Claude Code 读 settings.json 即时生效、无需重启进程，
/// 因此不存在对 Claude 构造 BeforeProcessStart 的场景。
struct ClaudeSyncBody;

impl ClientSync for ClaudeSyncBody {
    fn id(&self) -> ClientId {
        ClientId::Claude
    }

    fn sync(
        &self,
        ctx: &AppContext,
        trigger: &SyncTrigger,
        _material: SyncMaterial<'_>,
    ) -> AppResult<SyncOutcome> {
        match trigger {
            // 覆盖 live 前：无激活 / 自切跳过，其余先把旧激活快照同步到现场（缺口 C1）
            SyncTrigger::BeforeLiveOverwrite { target_id, .. } => {
                let Some(active) = ctx.database.active_claude_profile()? else {
                    return Ok(SyncOutcome::bare(SyncKind::NoActiveProfile));
                };
                if active == *target_id {
                    return Ok(SyncOutcome::bare(SyncKind::TargetMismatch));
                }
                ctx.sync_active_claude_settings()
            }
            // 打开编辑页/复制前：目标就是激活配置时才同步（缺口 C3）
            SyncTrigger::BeforeProfileRead { target_id, .. }
            | SyncTrigger::BeforeProfileClone { target_id, .. } => {
                if ctx.database.active_claude_profile()?.as_deref() != Some(target_id.as_str()) {
                    return Ok(SyncOutcome::bare(SyncKind::TargetMismatch));
                }
                ctx.sync_active_claude_settings()
            }
            // 被动刷新与捕获后：无激活直接跳过且不读 live 文件（冷启动预算）
            SyncTrigger::StateRefresh | SyncTrigger::AfterCapture { .. } => {
                if ctx.database.active_claude_profile()?.is_none() {
                    return Ok(SyncOutcome::bare(SyncKind::NoActiveProfile));
                }
                ctx.sync_active_claude_settings()
            }
            // Claude 无需重启进程，无人构造此时机；保守按 no-op 处理
            SyncTrigger::BeforeProcessStart { .. } => {
                Ok(SyncOutcome::bare(SyncKind::TargetMismatch))
            }
        }
    }
}

pub struct SyncRegistry {
    bodies: Vec<Box<dyn ClientSync>>,
}

impl SyncRegistry {
    /// 收割：首个执行体出错即中断剩余执行体并返回（被动路径自愈，下一轮补上；
    /// 调用方用 `?` 或 `let _` 决定传播，对齐原各触发点两种错误语义并存的现状）。
    /// 顺序 = 注册顺序（确定）。互斥由调用方保证：两个引擎的 apply（覆盖 live 的
    /// 临界段，含 BeforeLiveOverwrite 收割）都在 operation 锁内调用；被动路径走
    /// harvest_passive 让路；读/捕获路径（Read/Clone/AfterCapture）不持锁，
    /// 接受与切换的窄竞态（重构前即如此，SQLite 连接锁兜底单条写）。
    pub fn harvest(
        &self,
        ctx: &AppContext,
        trigger: &SyncTrigger,
        material: SyncMaterial<'_>,
    ) -> AppResult<()> {
        // 冷启动预算：被动回写落在 get_state 同步路径上，超阈值必须可见
        let started = std::time::Instant::now();
        for body in self
            .bodies
            .iter()
            .filter(|body| trigger.is_passive() || body.id() == trigger.client())
        {
            let outcome = body.sync(ctx, trigger, material)?;
            // 回写结局的唯一日志出口：unchanged 轮询降到 Trace（一轮两条的常驻噪音），
            // 其余常态走 Debug（release 自动消失），外部损坏/真失败各留一条 Warn，
            // 排查不必再靠排除法反推是哪道守卫拦的。profile 无可指时落 "-"。
            let kind = outcome.kind;
            let profile = outcome.profile.as_deref().unwrap_or("-");
            // target_id 仅主动时机存在；用前导空格拼段，被动轮询不落该字段。
            let target = trigger
                .target_id()
                .map(|id| format!(" target_id={id:?}"))
                .unwrap_or_default();
            let line = format!(
                "[sync.harvest] client={:?} profile={profile:?} trigger={}{} outcome={} reason={}",
                body.id().label(),
                trigger.token(),
                target,
                kind.outcome(),
                kind.reason(),
            );
            if let Some(failure_kind) = kind.failure_kind() {
                tauri_plugin_log::log::warn!(
                    "{line} failure_kind={failure_kind} msg={:?}",
                    kind.message()
                );
            } else if matches!(kind, SyncKind::Unchanged) {
                tauri_plugin_log::log::trace!("{line} msg={:?}", kind.message());
            } else {
                tauri_plugin_log::log::debug!("{line} msg={:?}", kind.message());
            }
        }
        let elapsed_ms = started.elapsed().as_millis();
        if elapsed_ms > 50 {
            tauri_plugin_log::log::warn!(
                "[sync.harvest] trigger={} latency_ms={} outcome=failure failure_kind=internal msg=\"回写耗时超过 50ms 预算\"",
                trigger.token(),
                elapsed_ms
            );
        }
        Ok(())
    }

    /// 被动收割（get_state 路径）：操作锁被切换/应用/重启占用就跳过本轮，
    /// 下一轮激活自愈；不阻塞 UI 刷新，不放大首屏延迟。失败不冒泡成 get_state 失败。
    pub fn harvest_passive(
        &self,
        ctx: &AppContext,
        trigger: &SyncTrigger,
        material: SyncMaterial<'_>,
    ) {
        match ctx.operation.try_lock() {
            Ok(_guard) => {
                if let Err(error) = self.harvest(ctx, trigger, material) {
                    // 内部错误也统一截断，与仓库日志规范一致（防 sqlite 报错内嵌意外长文本）
                    let brief: String = error.0.chars().take(160).collect();
                    tauri_plugin_log::log::warn!(
                        "[sync.harvest] outcome=failure failure_kind=internal error={:?} msg=\"被动回写失败，本轮跳过\"",
                        brief
                    );
                }
            }
            Err(_) => {
                tauri_plugin_log::log::debug!(
                    "[sync.harvest] trigger={} outcome=skipped reason=lock_busy msg=\"操作锁占用中，本轮跳过\"",
                    trigger.token()
                );
            }
        }
    }
}

static REGISTRY: OnceLock<SyncRegistry> = OnceLock::new();

pub fn registry() -> &'static SyncRegistry {
    REGISTRY.get_or_init(|| SyncRegistry {
        bodies: vec![Box::new(CodexSyncBody), Box::new(ClaudeSyncBody)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> (tempfile::TempDir, AppContext) {
        let home = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(home.path()).unwrap();
        paths.ensure().unwrap();
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();
        let context = AppContext::new(paths.clone()).unwrap();
        (home, context)
    }

    #[test]
    fn passive_harvest_converges_external_drift_into_active_snapshot() {
        let (_home, context) = test_context();
        let profile = context.codex_capture_profile("A").unwrap();
        context.codex_apply_profile(&profile.id).unwrap();
        // 外部改写 live：被动收割后激活快照必须收敛到最新内容
        std::fs::write(context.paths.codex_config(), "model = \"glm-5.4\"\n").unwrap();
        registry().harvest_passive(
            &context,
            &SyncTrigger::StateRefresh,
            SyncMaterial::default(),
        );
        let stored = context.database.codex_profile(&profile.id).unwrap();
        assert!(stored
            .payload
            .raw_config
            .as_deref()
            .unwrap_or_default()
            .contains("glm-5.4"));
        // 幂等：同内容再收割一次仍是 no-op，不报错
        registry().harvest_passive(
            &context,
            &SyncTrigger::StateRefresh,
            SyncMaterial::default(),
        );
    }

    /// 宽松读路径的三种非正常结局必须可辨且带名（守卫契约硬要求：LiveAbsent 与
    /// LiveUnreadable 分开、除两条 bare 外必须带出配置名）：读不了被报成解析失败，
    /// 或日志里看不出是哪个配置，都会把排查引去错误方向。
    #[test]
    fn loose_read_failures_are_distinguishable_and_carry_profile_name() {
        let (_home, context) = test_context();
        let profile = context.codex_capture_profile("A").unwrap();
        context.codex_apply_profile(&profile.id).unwrap();

        // live 不存在是正常态，但结局仍要带出配置名
        std::fs::remove_file(context.paths.codex_config()).unwrap();
        let outcome = sync_loose(&context, None).unwrap();
        assert_eq!(outcome.kind, SyncKind::LiveAbsent);
        assert_eq!(outcome.profile.as_deref(), Some("A"));

        // 读不了（权限/占用）：目录占位稳定复现非 NotFound 的 io 失败
        std::fs::create_dir(context.paths.codex_config()).unwrap();
        let outcome = sync_loose(&context, None).unwrap();
        assert_eq!(outcome.kind, SyncKind::LiveUnreadable);
        assert_eq!(outcome.profile.as_deref(), Some("A"));

        // 解析失败（被外部改坏）
        std::fs::remove_dir(context.paths.codex_config()).unwrap();
        std::fs::write(context.paths.codex_config(), "model = \n").unwrap();
        let outcome = sync_loose(&context, None).unwrap();
        assert_eq!(outcome.kind, SyncKind::LiveParseError);
        assert_eq!(outcome.profile.as_deref(), Some("A"));
    }

    /// 日志里的客户端名必须用产品名而不是内部枚举名：
    /// 排查时看的是 "Claude Code" / "Codex"，不是 Debug 打出来的 "Claude"。
    #[test]
    fn client_log_label_uses_product_name() {
        assert_eq!(ClientId::Codex.label(), "Codex");
        assert_eq!(ClientId::Claude.label(), "Claude Code");
    }

    #[test]
    fn harvest_without_active_profile_is_a_noop() {
        let (_home, context) = test_context();
        // 无激活：被动与客户端域时机都是 no-op，不读 live、不写库、不报错
        registry()
            .harvest(
                &context,
                &SyncTrigger::StateRefresh,
                SyncMaterial::default(),
            )
            .unwrap();
        registry()
            .harvest(
                &context,
                &SyncTrigger::BeforeLiveOverwrite {
                    client: ClientId::Codex,
                    target_id: "any".into(),
                },
                SyncMaterial::default(),
            )
            .unwrap();
        assert!(context.database.codex_profiles().unwrap().is_empty());
    }
}
