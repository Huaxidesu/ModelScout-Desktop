/**
 * ============================================================================
 * ModelScout —— Tauri 命令层
 * ============================================================================
 *
 * 【职责】
 * 把 `probe.rs` 的能力暴露为 6 个 `#[tauri::command]`。
 * 本文件是前端 `invoke()` 的唯一对接面。
 *
 * 【命令清单（必须与 src-tauri/src/lib.rs 的 generate_handler! 完全一致）】
 *   1. list_models
 *   2. ping_connection
 *   3. start_probe
 *   4. stop_probe
 *   5. get_probe_state
 *   6. save_config_file
 *
 * 【Tauri v2 要点】
 *   - 命令用 `async fn`，返回 `Result<T, E>`，其中 E 需实现 Serialize
 *   - 参数名在前端以 camelCase 传入（Tauri 会自动做 snake_case 映射）
 *   - `AppHandle` / `State` 作为参数注入，由框架提供
 * ============================================================================
 */

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

use crate::dictionary;
use crate::error::{CommandError, ProbeError, ProbeErrorKind};
use crate::models::{
    ProbeConfig, ProbeRunSummary, ProbeSnapshot, ProbeStage, ProbeStatus, SaveConfigRequest,
    SaveConfigResult, PingResult,
};
use crate::probe::{
    self, ApiClient, EngineOptions, RunState,
};

/* ===========================================================================
 * 应用全局状态
 * ========================================================================= */

/// 被 Tauri 托管的全局状态。
///
/// 用 `Mutex<Option<Arc<RunState>>>` 而非直接 `Option<Arc<RunState>>` 的原因：
/// 命令是 `&State<AppState>` 的不可变引用，要在其中替换内部值必须经过内部可变性。
/// 这里的临界区极短（只是 clone 一个 Arc 或写入一个 Option），
/// 不会成为并发瓶颈。
#[derive(Default)]
pub struct AppState {
    /// 当前正在运行的任务状态；空闲时为 None
    pub current: Mutex<Option<Arc<RunState>>>,
    /// 最近一次运行的整体状态（用于 get_probe_state 在空闲时也能回答）
    pub last_status: Mutex<ProbeStatus>,
}

impl AppState {
    /// 创建一个空状态。
    ///
    /// `Default` 派生会把 `last_status` 初始化为 `ProbeStatus::Idle`，
    /// 但为可读性这里显式提供一个构造函数。
    pub fn new() -> Self {
        Self {
            current: Mutex::new(None),
            last_status: Mutex::new(ProbeStatus::Idle),
        }
    }

    /// 读取当前运行状态。
    pub async fn snapshot_state(&self) -> Option<Arc<RunState>> {
        self.current.lock().await.clone()
    }

    /// 写入/清除当前运行状态。
    pub async fn set_state(&self, value: Option<Arc<RunState>>) {
        *self.current.lock().await = value;
    }

    /// 读取最近一次整体状态。
    pub async fn get_last_status(&self) -> ProbeStatus {
        *self.last_status.lock().await
    }

    /// 写入最近一次整体状态。
    pub async fn set_last_status(&self, value: ProbeStatus) {
        *self.last_status.lock().await = value;
    }
}

/* ===========================================================================
 * 1. list_models —— 仅拉取模型列表
 * ========================================================================= */

/// 拉取 `/models` 返回的模型清单，不做任何能力探测。
///
/// 【用途】
/// 让用户在正式探测前先看到「这个端点上有哪些模型」，
/// 从而可以用 `onlyModels` 缩小探测范围，节省时间与额度。
///
/// # 参数
/// - `config`: 探测配置（此命令只用到 baseUrl / apiKey / timeoutSecs）
///
/// # 返回
/// `ModelListResult`；失败时返回分类后的 `CommandError`。
#[tauri::command]
pub async fn list_models(
    config: ProbeConfig,
) -> Result<crate::models::ModelListResult, CommandError> {
    let config = config.normalized();
    config.validate().map_err(CommandError::invalid_input)?;
    // 把「环境变量 / 凭据管理器」来源解析为明文 Key（明文来源则原样返回）。
    // 解析失败（变量未设置、凭据不存在等）按参数问题归类，提示用户修正。
    let config = config
        .with_resolved_key()
        .map_err(CommandError::invalid_input)?;

    let client = ApiClient::new(&config).map_err(|err| CommandError::from_probe(&err))?;

    let started = std::time::Instant::now();
    match probe::fetch_model_list(&client).await {
        Ok((models, elapsed_ms)) => Ok(crate::models::ModelListResult {
            models,
            errors: Vec::new(),
            elapsed_ms,
        }),
        Err(err) => Err(CommandError::from_probe(&err)),
    }
    .map(|mut result| {
        // 若上面的 Ok 分支已给出 elapsed_ms，这里做一次校正，
        // 保证耗时反映整个命令的墙钟时间。
        result.elapsed_ms = result.elapsed_ms.max(started.elapsed().as_millis() as u64);
        result
    })
}

/* ===========================================================================
 * 2. ping_connection —— 连通性 + 鉴权预检
 * ========================================================================= */

/// 构造一个「预检失败」的 `PingResult`。
///
/// 预检命令的契约是**永不返回 Err**：无论参数不合法、密钥无法解析还是
/// 网络/鉴权失败，都封装成 `PingResult.error` 交给前端渲染诊断卡片。
fn ping_failure(message: impl Into<String>, resolved_url: String) -> PingResult {
    PingResult {
        ok: false,
        http_status: 0,
        error: Some(ProbeError::new(
            ProbeErrorKind::Unknown,
            message,
            0,
            ProbeStage::Listing,
        )),
        elapsed_ms: 0,
        model_count: 0,
        resolved_url,
    }
}

/// 连通性与鉴权预检。
///
/// 【与 list_models 的区别】
/// 本命令**永不返回 Err**：所有失败（参数不合法、密钥来源无法解析、
/// 网络或鉴权失败）都被归类塞进 `PingResult.error`，
/// 让前端可以直接渲染一个「诊断卡片」，而不需要走 catch 分支。
///
/// # 参数
/// - `config`: 探测配置。本命令只用到 `baseUrl`、密钥（明文或其来源）
///   与 `timeoutSecs`；并发数与上限探测开关会被强制覆盖为
///   「单并发、不跑上限探测」。
#[tauri::command]
pub async fn ping_connection(config: ProbeConfig) -> Result<PingResult, CommandError> {
    // 预检固定单并发、不跑上限探测，其余字段沿用调用方传入的配置
    let config = ProbeConfig {
        concurrency: 1,
        probe_limits: false,
        ..config.normalized()
    };

    let resolved_url = config.base_url.clone();

    // 参数校验失败也走 PingResult 而非 Err —— 保持「预检命令不抛错」的契约
    if let Err(message) = config.validate() {
        return Ok(ping_failure(message, resolved_url));
    }

    // 密钥来源解析失败（环境变量未设置 / 凭据不存在）同样走 PingResult
    let config = match config.with_resolved_key() {
        Ok(resolved) => resolved,
        Err(message) => return Ok(ping_failure(message, resolved_url)),
    };

    let client = match ApiClient::new(&config) {
        Ok(client) => client,
        Err(err) => {
            return Ok(PingResult {
                ok: false,
                http_status: 0,
                error: Some(err),
                elapsed_ms: 0,
                model_count: 0,
                resolved_url,
            })
        }
    };

    let started = std::time::Instant::now();
    match probe::fetch_model_list(&client).await {
        Ok((models, _)) => Ok(PingResult {
            ok: true,
            http_status: 200,
            error: None,
            elapsed_ms: started.elapsed().as_millis() as u64,
            model_count: models.len(),
            resolved_url,
        }),
        Err(err) => Ok(PingResult {
            ok: false,
            http_status: err.http_status,
            error: Some(err),
            elapsed_ms: started.elapsed().as_millis() as u64,
            model_count: 0,
            resolved_url,
        }),
    }
}

/* ===========================================================================
 * 3. start_probe —— 启动完整探测流水线
 * ========================================================================= */

/// 启动完整的并发探测流水线。
///
/// 【异步语义】
/// 本命令会**等待整轮探测跑完**才返回。之所以这样设计而不是「立刻返回、
/// 靠事件推送」：
///   1. 前端需要拿到最终汇总用于生成配置，事件只是过程展示；
///   2. Tauri 的 async command 在独立任务中执行，不会阻塞 UI 线程；
///   3. 请求/响应语义比「启动 + 轮询」更简单，出错路径更少。
/// 在此期间，全部进度通过 5 个 `probe://*` 事件实时推送。
///
/// # 参数
/// - `config`: 完整探测配置
///
/// # 返回
/// `ProbeRunSummary`（含全部模型结果）；出现致命错误时返回 `CommandError`。
#[tauri::command]
pub async fn start_probe(
    app: AppHandle,
    state: State<'_, AppState>,
    config: ProbeConfig,
) -> Result<ProbeRunSummary, CommandError> {
    let config = config.normalized();
    config.validate().map_err(CommandError::invalid_input)?;
    // 解析密钥来源（环境变量 / 凭据管理器 → 明文）。
    // 放在互斥检查之前：配置本身有问题时应当立刻失败，不占用「已有任务」的判断。
    let config = config
        .with_resolved_key()
        .map_err(CommandError::invalid_input)?;

    // ---- 并发互斥：若已有任务在跑，直接拒绝 ----
    {
        let guard = state.current.lock().await;
        if let Some(existing) = guard.as_ref() {
            if !existing.is_cancelled() {
                return Err(CommandError::new(
                    ProbeErrorKind::Unknown,
                    format!(
                        "已有探测任务在运行（运行 ID {}），请先点击「停止探测」再重新开始。",
                        existing.run_id
                    ),
                    0,
                ));
            }
        }
    }

    let run_id = probe::new_run_id();

    // 记录配置摘要（Key 已脱敏），便于排障
    eprintln!("[ModelScout] 启动探测 {}", probe::describe_config(&config));

    let client = ApiClient::new(&config).map_err(|err| CommandError::from_probe(&err))?;

    // ---- 阶段 A：拉取模型列表 ----
    let list_started = std::time::Instant::now();
    let (mut briefs, _list_elapsed) = match probe::fetch_model_list(&client).await {
        Ok(pair) => pair,
        Err(err) => {
            // 致命错误：列表都拿不到，整轮无法进行
            probe::emit_fatal(&app, &run_id, err.clone(), ProbeStage::Listing);
            probe::emit_finished(
                &app,
                &run_id,
                ProbeStatus::Failed,
                0,
                0,
                list_started.elapsed().as_millis() as u64,
                vec![err.clone()],
            );
            state.set_last_status(ProbeStatus::Failed).await;
            return Err(CommandError::from_probe(&err));
        }
    };

    // ---- 应用模型白名单过滤 ----
    if !config.only_models.is_empty() {
        let wanted = &config.only_models;
        briefs.retain(|b| wanted.iter().any(|w| w == &b.id));
    }

    let total = briefs.len();

    // ---- 阶段 B：建立运行状态并保存到全局 ----
    let run_state = RunState::new(run_id.clone(), total);
    state.set_state(Some(run_state.clone())).await;
    state.set_last_status(ProbeStatus::Running).await;

    // ---- 阶段 C：执行并发引擎 ----
    // 引擎通过「事件端口」发事件，生产环境用 TauriSink 转发到前端；
    // 测试可换成记录型 sink，从而无需启动 Tauri 应用（见 D-37）。
    let outcome = probe::run_engine(
        probe::TauriSink(app.clone()),
        run_state.clone(),
        client,
        briefs,
        config.concurrency,
        EngineOptions {
            probe_limits: config.probe_limits,
        },
    )
    .await;

    // ---- 阶段 D：收尾 ----
    let status = outcome.status;
    // 任务结束后清空 current，但保留 last_status
    state.set_state(None).await;
    state.set_last_status(status).await;

    let summary = probe::build_summary(&run_id, outcome);
    probe::emit_finished(
        &app,
        &run_id,
        status,
        summary.total_models,
        total,
        summary.elapsed_ms,
        summary.errors.clone(),
    );

    eprintln!(
        "[ModelScout] 探测结束 runId={} status={:?} 成功={} 总耗时={}ms",
        run_id, status, summary.total_models, summary.elapsed_ms
    );

    Ok(summary)
}

/* ===========================================================================
 * 4. stop_probe —— 中止当前探测
 * ========================================================================= */

/// 请求中止当前正在运行的探测任务。
///
/// 【中止语义（诚实说明）】
/// 「中止」是**协作式**的，不是立即杀死：
///   - 尚未开始的任务：会被直接丢弃（buffer_unordered 流的 drop 语义）
///   - 正在等待网络响应的任务：无法取消已发出的请求，
///     它们会在各自超时后结束；期间结果会被忽略
///   - 因此返回值里的 `completed` 可能仍会在此后略微增长
///
/// # 返回
/// 返回中止时刻的状态快照。无任务运行时返回 `Idle` 快照（不报错）。
#[tauri::command]
pub async fn stop_probe(
    state: State<'_, AppState>,
) -> Result<ProbeSnapshot, CommandError> {
    let current = state.snapshot_state().await;

    match current {
        Some(run_state) => {
            run_state.request_cancel();
            state.set_last_status(ProbeStatus::Cancelling).await;
            eprintln!(
                "[ModelScout] 收到中止请求 runId={} 已完成 {}/{}",
                run_state.run_id,
                run_state.completed.load(std::sync::atomic::Ordering::SeqCst),
                run_state.total.load(std::sync::atomic::Ordering::SeqCst)
            );
            let snapshot = probe::build_snapshot(Some(&run_state), ProbeStatus::Cancelling).await;
            Ok(snapshot)
        }
        None => {
            // 无任务在跑：返回空闲快照，不视作错误
            let snapshot = probe::build_snapshot(None, ProbeStatus::Idle).await;
            Ok(snapshot)
        }
    }
}

/* ===========================================================================
 * 5. get_probe_state —— 查询实时快照
 * ========================================================================= */

/// 查询当前探测快照。
///
/// 【用途】
/// - 前端窗口刷新 / 面板重新挂载时重同步状态
/// - 事件丢失（例如 DevTools 重连）后的回退轮询
///
/// # 返回
/// 任意时刻都返回 `Ok`，空闲时是 `status = idle` 的空快照。
#[tauri::command]
pub async fn get_probe_state(
    state: State<'_, AppState>,
) -> Result<ProbeSnapshot, CommandError> {
    let current = state.snapshot_state().await;
    let last_status = state.get_last_status().await;

    // 有任务在跑：用 Running；已结束：用最近状态
    let effective_status = if current.is_some() {
        ProbeStatus::Running
    } else {
        last_status
    };

    Ok(probe::build_snapshot(current.as_ref(), effective_status).await)
}

/* ===========================================================================
 * 6. save_config_file —— 把生成的配置写入磁盘
 * ========================================================================= */

/// 把生成的配置文本写入用户指定/默认位置。
///
/// 【设计取舍】
/// 官方推荐做法是前端用 `@tauri-apps/plugin-dialog` 弹保存框，
/// 但那需要额外引入 dialog 插件与权限。为了把依赖面控制到最小
/// （决策日志 D-08），这里采用「写入应用数据目录下的 exports/ 子目录」
/// 的方案：
///   - Windows: %APPDATA%/com.huaxidesu.modelscout/exports/
///   - Linux  : ~/.local/share/com.huaxidesu.modelscout/exports/
///
/// 不覆盖已存在文件：若同名文件已存在则追加 `-1` `-2` 等后缀，
/// 避免用户前一次生成的配置被静默覆盖。
///
/// # 参数
/// - `request`: `{ fileName, content }`
///
/// # 返回
/// `{ saved: true, path: "<绝对路径>" }`
#[tauri::command]
pub async fn save_config_file(
    app: AppHandle,
    request: SaveConfigRequest,
) -> Result<SaveConfigResult, CommandError> {
    // ---- 文件名安全校验：禁止路径穿越 ----
    let raw_name = request.file_name.trim();
    if raw_name.is_empty() {
        return Err(CommandError::invalid_input("文件名不能为空。"));
    }
    if raw_name.contains("..") || raw_name.contains('/') || raw_name.contains('\\') {
        return Err(CommandError::invalid_input(format!(
            "文件名不合法（不允许包含路径分隔符或 ..）：{raw_name}"
        )));
    }

    // ---- 解析导出目录 ----
    let base_dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|err| CommandError::internal(format!("无法获取应用数据目录：{err}")))?;

    let export_dir = base_dir.join("exports");
    std::fs::create_dir_all(&export_dir)
        .map_err(|err| CommandError::internal(format!("无法创建导出目录：{err}")))?;

    // ---- 选择不冲突的文件名 ----
    let (stem, ext) = match raw_name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (raw_name.to_string(), String::new()),
    };

    let mut target = export_dir.join(format!("{stem}{ext}"));
    let mut counter = 1usize;
    while target.exists() {
        target = export_dir.join(format!("{stem}-{counter}{ext}"));
        counter += 1;
        if counter > 999 {
            return Err(CommandError::internal(
                "同名文件过多（超过 999 个），请先清理导出目录。",
            ));
        }
    }

    // ---- 写入 ----
    std::fs::write(&target, request.content.as_bytes())
        .map_err(|err| CommandError::internal(format!("写入文件失败：{err}")))?;

    let path = target.to_string_lossy().to_string();
    eprintln!("[ModelScout] 配置已导出：{path}");

    Ok(SaveConfigResult {
        saved: true,
        path,
    })
}

/* ===========================================================================
 * 7. get_dictionary_info —— 字典自检（辅助命令，非任务书要求但有助于排障）
 * ========================================================================= */

/// 返回字典条目数与版本信息，便于前端在「设置/关于」处展示数据新鲜度。
#[tauri::command]
pub async fn get_dictionary_info() -> Result<Value, CommandError> {
    Ok(serde_json::json!({
        "entryCount": dictionary::entry_count(),
    }))
}

/* ---------------------------------------------------------------------------
 * 单元测试（仅覆盖不依赖 Tauri 运行时的纯逻辑）
 * ------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::ping_failure;
    use crate::models::{ApiKeySource, ProbeConfig};

    /// 构造一份「明文模式」的测试配置，避免每处字面量都重复三个来源字段。
    fn plain_config(base_url: &str, api_key: &str) -> ProbeConfig {
        ProbeConfig {
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            concurrency: 3,
            timeout_secs: 30,
            probe_limits: true,
            only_models: vec![],
            api_key_source: ApiKeySource::Plain,
            api_key_env_var: String::new(),
            api_key_credential: String::new(),
        }
    }

    #[test]
    fn probe_config_validation_rejects_empty_fields() {
        assert!(plain_config("   ", "sk-x").normalized().validate().is_err());
        assert!(plain_config("https://api.example.com/v1", "  ")
            .normalized()
            .validate()
            .is_err());
        assert!(plain_config("https://api.example.com/v1", "sk-x")
            .normalized()
            .validate()
            .is_ok());
    }

    #[test]
    fn ping_failure_never_returns_err_but_carries_message() {
        // 预检命令契约：失败也必须以 PingResult.error 表达，而不是 Err
        let result = ping_failure("环境变量 FOO 未设置。".to_string(), "https://x/v1".to_string());
        assert!(!result.ok);
        assert_eq!(result.error.as_ref().unwrap().message, "环境变量 FOO 未设置。");
        assert_eq!(result.resolved_url, "https://x/v1");
    }

    #[test]
    fn file_name_sanitization_rules() {
        // 复制自 save_config_file 的校验逻辑，作为回归测试
        fn sanitize(name: &str) -> Result<String, String> {
            let raw = name.trim();
            if raw.is_empty() {
                return Err("空".to_string());
            }
            if raw.contains("..") || raw.contains('/') || raw.contains('\\') {
                return Err("非法".to_string());
            }
            Ok(raw.to_string())
        }

        assert!(sanitize("opencode.json").is_ok());
        assert!(sanitize("deepseek-hardness.toml").is_ok());
        assert!(sanitize("../../etc/passwd").is_err());
        assert!(sanitize("dir/file.json").is_err());
        assert!(sanitize("dir\\file.json").is_err());
        assert!(sanitize("   ").is_err());
    }
}
