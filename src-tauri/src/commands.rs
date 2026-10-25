//! Tauri 命令层：把探测引擎与字典的能力暴露为 `#[tauri::command]`，
//! 是前端 `invoke()` 的唯一对接面。
//!
//! - 命令清单：`list_models` / `ping_connection` / `start_probe` /
//!   `stop_probe` / `get_probe_state` / `save_config_file` /
//!   `get_dictionary_info`，必须与 `lib.rs` 的 `generate_handler!`
//!   注册列表完全一致；
//! - 命令使用 `async fn`，返回 `Result<T, E>`（E 需实现 `Serialize`）；
//! - 参数名由前端以 camelCase 传入（Tauri 自动映射 snake_case）；
//! - `AppHandle` / `State` 作为参数注入，由框架提供。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

use crate::dictionary;
use crate::error::{CommandError, ProbeError, ProbeErrorKind};
use crate::models::{
    EnvSecretInfo, EnvSecretStatus, EnvSecretWriteResult, ProbeConfig, ProbeRunSummary,
    ProbeSnapshot, ProbeStage, ProbeStatus, SaveConfigRequest, SaveConfigResult, PingResult,
    ToolConfigFile, ToolConfigScan, ToolConfigWriteResult,
};
use crate::probe::{
    self, ApiClient, EngineOptions, RunState,
};

// ---- 应用全局状态 ----

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

// ---- list_models：仅拉取模型列表 ----

/// 拉取 `/models` 返回的模型清单，不做任何能力探测。
///
/// 用于在正式探测前查看端点上有哪些模型，配合 `onlyModels` 缩小探测范围，
/// 节省时间与额度。
///
/// - `config`：探测配置（本命令只使用 baseUrl / apiKey / timeoutSecs）；
/// - 返回 `ModelListResult`，失败时返回分类后的 `CommandError`。
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

// ---- ping_connection：连通性 + 鉴权预检 ----

/// 构造一个「预检失败」的 `PingResult`。
///
/// 预检命令的契约是永不返回 `Err`：无论参数不合法、密钥无法解析还是
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
/// 与 `list_models` 的区别：本命令永不返回 `Err`，所有失败（参数不合法、
/// 密钥来源无法解析、网络或鉴权失败）都归类放入 `PingResult.error`，
/// 前端可直接渲染诊断卡片，无需走 catch 分支。
///
/// - `config`：探测配置。本命令只使用 `baseUrl`、密钥（明文或其来源）与
///   `timeoutSecs`；并发数与上限探测开关被强制覆盖为
///   单并发、不执行上限探测。
#[tauri::command]
pub async fn ping_connection(config: ProbeConfig) -> Result<PingResult, CommandError> {
    // 预检固定单并发、不执行上限探测，其余字段沿用调用方传入的配置
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

// ---- start_probe：启动完整探测流水线 ----

/// 启动完整的并发探测流水线。
///
/// 本命令等待整轮探测完成后才返回，而非「立刻返回、靠事件推送」：
/// 1. 前端需要最终汇总用于生成配置，事件只承担过程展示；
/// 2. Tauri 的 async command 在独立任务中执行，不阻塞 UI 线程；
/// 3. 请求/响应语义比「启动 + 轮询」更简单，出错路径更少。
/// 运行期间全部进度通过 5 个 `probe://*` 事件实时推送。
///
/// - `config`：完整探测配置；
/// - 返回 `ProbeRunSummary`（含全部模型结果）；致命错误时返回 `CommandError`。
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

    // ---- 并发互斥：若有任务正在运行，直接拒绝 ----
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
    // 引擎通过「事件端口」发事件：生产环境用 `TauriSink` 转发到前端，
    // 测试可用记录型 sink，无需启动 Tauri 应用。
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

// ---- stop_probe：中止当前探测 ----

/// 请求中止当前正在运行的探测任务。
///
/// 中止是协作式的，并非立即终止：
/// - 尚未开始的任务被直接丢弃（`buffer_unordered` 流的 drop 语义）；
/// - 正在等待网络响应的任务无法取消已发出的请求，它们在各自超时后结束，
///   期间结果被忽略；
/// - 因此返回值中的 `completed` 仍可能在此后略微增长。
///
/// 返回中止时刻的状态快照；无任务运行时返回 `Idle` 快照（不报错）。
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
            // 无任务运行：返回空闲快照，不视作错误
            let snapshot = probe::build_snapshot(None, ProbeStatus::Idle).await;
            Ok(snapshot)
        }
    }
}

// ---- get_probe_state：查询实时快照 ----

/// 查询当前探测快照。
///
/// 用于前端窗口刷新 / 面板重新挂载时重同步状态，以及事件丢失
/// （例如 DevTools 重连）后的回退轮询。任意时刻都返回 `Ok`，
/// 空闲时是 `status = idle` 的空快照。
#[tauri::command]
pub async fn get_probe_state(
    state: State<'_, AppState>,
) -> Result<ProbeSnapshot, CommandError> {
    let current = state.snapshot_state().await;
    let last_status = state.get_last_status().await;

    // 有任务在运行：用 Running；已结束：用最近状态
    let effective_status = if current.is_some() {
        ProbeStatus::Running
    } else {
        last_status
    };

    Ok(probe::build_snapshot(current.as_ref(), effective_status).await)
}

// ---- save_config_file：把生成的配置写入磁盘 ----

/// 把生成的配置文本写入用户指定/默认位置。
///
/// 未采用官方推荐的 `@tauri-apps/plugin-dialog` 保存框方案：那需要额外引入
/// dialog 插件与权限；为把依赖面控制到最小，改为写入应用数据目录下的
/// exports/ 子目录：
/// - Windows: %APPDATA%/com.huaxidesu.modelscout/exports/
/// - Linux  : ~/.local/share/com.huaxidesu.modelscout/exports/
///
/// 不覆盖已存在文件：同名文件已存在时追加 `-1`、`-2` 等后缀，
/// 避免用户先前生成的配置被静默覆盖。
///
/// - `request`：`{ fileName, content }`；
/// - 返回 `{ saved: true, path: "<绝对路径>" }`。
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

// ---- get_dictionary_info：字典自检（辅助命令） ----

/// 返回字典条目数与版本信息，便于前端在「设置/关于」处展示数据新鲜度。
#[tauri::command]
pub async fn get_dictionary_info() -> Result<Value, CommandError> {
    Ok(serde_json::json!({
        "entryCount": dictionary::entry_count(),
    }))
}

// ---- inspect_tool_config / write_tool_config：一键填写 ----

/// 一键填写读取的配置文件大小上限（1 MiB）。
///
/// opencode / dsh 的配置都是几十 KB 级；超限说明这个路径指向的多半不是
/// 预期的配置文件，读进来也无法在 WebView 里高效渲染。
const TOOL_CONFIG_MAX_BYTES: u64 = 1024 * 1024;

/// 允许一键填写写入的扩展名白名单。
const TOOL_CONFIG_EXTENSIONS: [&str; 5] = ["json", "jsonc", "yml", "yaml", "toml"];

/// 解析用户主目录（Windows 用 USERPROFILE，其余用 HOME）。
fn resolve_home_dir() -> Result<PathBuf, CommandError> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| CommandError::internal("无法确定用户主目录（环境变量缺失）。".to_string()))
}

/// 展开路径开头的 `~` 与 `~\` / `~/`。
fn expand_tilde(raw: &str, home: &std::path::Path) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = trimmed.strip_prefix("~/").or_else(|| trimmed.strip_prefix("~\\")) {
        return home.join(rest);
    }
    PathBuf::from(trimmed)
}

/// opencode 的默认配置路径（`~/.config/opencode/opencode.json`）。
///
/// opencode 全平台都以 `~/.config` 为根（官方文档只给出这一条路径），
/// 设置了 `XDG_CONFIG_HOME` 时优先采用。已存在 `opencode.jsonc` 而没有
/// `opencode.json` 时跟随用户的实际选择；都不存在时默认创建 `.json`。
fn default_opencode_config_path(home: &std::path::Path) -> PathBuf {
    let config_root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let dir = config_root.join("opencode");
    let json = dir.join("opencode.json");
    if json.exists() {
        return json;
    }
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.exists() {
        return jsonc;
    }
    json
}

/// DeepSeek Harness（dsh）补丁文件的默认路径。
///
/// 规则（依据 dsh 官方文档与 dsh-home-paths 解析器）：
///   - 根目录：`$DSH_HOME`（设了且非空就用）否则 `~/.dsh`；
///   - 文件：`<根>/profiles/<profile>/cordis.patch.yml`；
///   - profile 探测：profiles 目录下已有补丁文件的 profile 里优先 `web`
///     （`dsh web` 标准启动的 profile 名），否则取字母序第一个；
///   - 都没有：按 `web` 创建。
fn default_dsh_patch_path(home: &std::path::Path) -> PathBuf {
    let root = std::env::var_os("DSH_HOME")
        .map(|v| PathBuf::from(v.to_string_lossy().to_string()))
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| expand_tilde(&p.to_string_lossy(), home))
        .unwrap_or_else(|| home.join(".dsh"));

    let profiles = root.join("profiles");
    let patch_name = "cordis.patch.yml";

    if let Ok(entries) = std::fs::read_dir(&profiles) {
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join(patch_name).is_file())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        if !names.is_empty() {
            names.sort();
            names.sort_by_key(|n| (*n != "web") as u8); // web 稳定排到最前
            return profiles.join(&names[0]).join(patch_name);
        }
    }
    profiles.join("web").join(patch_name)
}

/// 把 `tool` 名解析成默认路径。
fn default_tool_config_path(tool: &str, home: &std::path::Path) -> Result<PathBuf, CommandError> {
    match tool {
        "opencode" => Ok(default_opencode_config_path(home)),
        "dsh" => Ok(default_dsh_patch_path(home)),
        other => Err(CommandError::invalid_input(format!(
            "未知的目标工具：{other}（支持 opencode / dsh）"
        ))),
    }
}

/// 读取目标工具的配置文件，供「一键填写」生成左右对比预览。
///
/// - `tool`：`opencode` / `dsh`，决定默认路径；
/// - `custom_path`：用户在界面里改过的路径（支持 `~`），优先生效。
///
/// 文件不存在**不是错误**：`exists=false` + 空 content 返回，前端据此走
///「空文件补全」分支。
#[tauri::command]
pub async fn inspect_tool_config(
    tool: String,
    custom_path: Option<String>,
) -> Result<ToolConfigFile, CommandError> {
    let home = resolve_home_dir()?;

    let path = match custom_path {
        Some(raw) if !raw.trim().is_empty() => {
            let expanded = expand_tilde(&raw, &home);
            // 相对路径以当前工作目录补全，保证返回给前端的是绝对路径
            if expanded.is_absolute() {
                expanded
            } else {
                std::env::current_dir()
                    .map_err(|err| CommandError::internal(format!("无法获取工作目录：{err}")))?
                    .join(expanded)
            }
        }
        _ => default_tool_config_path(&tool, &home)?,
    };

    if !path.exists() {
        return Ok(ToolConfigFile {
            path: path.to_string_lossy().to_string(),
            exists: false,
            content: String::new(),
        });
    }
    if !path.is_file() {
        return Err(CommandError::invalid_input(format!(
            "目标路径不是文件：{}",
            path.display()
        )));
    }

    let size = std::fs::metadata(&path)
        .map_err(|err| CommandError::internal(format!("读取文件信息失败：{err}")))?
        .len();
    if size > TOOL_CONFIG_MAX_BYTES {
        return Err(CommandError::invalid_input(format!(
            "文件超过 1 MiB（{} 字节），不像一份配置文件；请检查路径。",
            size
        )));
    }

    let content = std::fs::read_to_string(&path).map_err(|err| {
        CommandError::internal(format!(
            "读取 {} 失败：{err}（文件必须是 UTF-8 文本）",
            path.display()
        ))
    })?;

    Ok(ToolConfigFile {
        path: path.to_string_lossy().to_string(),
        exists: true,
        content,
    })
}

/// 把「一键填写」确认后的内容写入目标配置文件。
///
/// 安全约束：
///   - 路径必须解析为绝对路径，且扩展名在白名单内（防误写任意文件）；
///   - 覆盖已有文件前创建 `<原名>.modelscout.bak` 备份（可经 `backup`
///     关闭，备份文件本身会被同名覆盖，不累积）。
#[tauri::command]
pub async fn write_tool_config(
    path: String,
    content: String,
    backup: bool,
) -> Result<ToolConfigWriteResult, CommandError> {
    let home = resolve_home_dir()?;
    let expanded = expand_tilde(&path, &home);
    let path = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|err| CommandError::internal(format!("无法获取工作目录：{err}")))?
            .join(expanded)
    };

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !TOOL_CONFIG_EXTENSIONS.contains(&ext.as_str()) {
        return Err(CommandError::invalid_input(format!(
            "目标文件的扩展名必须是 {} 之一：{}",
            TOOL_CONFIG_EXTENSIONS.join(" / "),
            path.display()
        )));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| CommandError::internal(format!("无法创建目录 {}: {err}", parent.display())))?;
    }

    // 覆盖前备份（仅当文件已存在）
    let mut backup_path: Option<String> = None;
    if backup && path.is_file() {
        let bak = path.with_extension(format!(
            "{}.modelscout.bak",
            path.extension().and_then(|e| e.to_str()).unwrap_or("")
        ));
        std::fs::copy(&path, &bak).map_err(|err| {
            CommandError::internal(format!("创建备份失败：{err}"))
        })?;
        backup_path = Some(bak.to_string_lossy().to_string());
    }

    let bytes = content.len() as u64;
    std::fs::write(&path, content.as_bytes())
        .map_err(|err| CommandError::internal(format!("写入 {} 失败：{err}", path.display())))?;

    eprintln!(
        "[ModelScout] 一键填写已写入：{}（{} 字节，备份：{}）",
        path.display(),
        bytes,
        backup_path.as_deref().unwrap_or("无")
    );

    Ok(ToolConfigWriteResult {
        path: path.to_string_lossy().to_string(),
        backup_path,
        bytes_written: bytes,
    })
}

/// 读取一个配置文件为 `ToolConfigFile`；不存在或读取失败返回 None（跳过）。
fn read_config_file_quietly(path: &std::path::Path) -> Option<ToolConfigFile> {
    if !path.is_file() {
        return None;
    }
    let size = std::fs::metadata(path).ok()?.len();
    if size > TOOL_CONFIG_MAX_BYTES {
        eprintln!("[ModelScout] 配置扫描跳过超限文件：{}（{size} 字节）", path.display());
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    Some(ToolConfigFile {
        path: path.to_string_lossy().to_string(),
        exists: true,
        content,
    })
}

/// 扫描本机的 OpenCode 与 dsh 配置文件（配置管理页「一键识别」的数据源）。
///
/// 与 `inspect_tool_config` 的区别：那个按工具返回**单个**目标文件（一键
/// 填写用），这里返回**全部**能找到的文件 —— OpenCode 的 json/jsonc 都给，
/// dsh 的每个 profile 各一份。读取失败/超限的文件跳过，不让整次扫描失败。
#[tauri::command]
pub async fn scan_tool_configs() -> Result<ToolConfigScan, CommandError> {
    let home = resolve_home_dir()?;

    // ---- OpenCode：json 与 jsonc 并列存在时都返回 ----
    let config_root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let opencode_dir = config_root.join("opencode");
    let mut opencode = Vec::new();
    for name in ["opencode.json", "opencode.jsonc"] {
        if let Some(file) = read_config_file_quietly(&opencode_dir.join(name)) {
            opencode.push(file);
        }
    }

    // ---- dsh：每个 profile 的 cordis.patch.yml（web 优先，名称排序）----
    let dsh_root = std::env::var_os("DSH_HOME")
        .map(|v| PathBuf::from(v.to_string_lossy().to_string()))
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| expand_tilde(&p.to_string_lossy(), &home))
        .unwrap_or_else(|| home.join(".dsh"));
    let mut dsh = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dsh_root.join("profiles")) {
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("cordis.patch.yml").is_file())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        names.sort();
        names.sort_by_key(|n| (*n != "web") as u8);
        for name in names {
            let path = dsh_root.join("profiles").join(&name).join("cordis.patch.yml");
            if let Some(file) = read_config_file_quietly(&path) {
                dsh.push(file);
            }
        }
    }

    eprintln!(
        "[ModelScout] 配置扫描：OpenCode {} 个文件，dsh {} 个文件",
        opencode.len(),
        dsh.len()
    );

    Ok(ToolConfigScan { opencode, dsh })
}

// ---- env_secret_list / env_secret_set / env_secret_delete：环境变量密钥管理 ----

/// 环境变量值写入用户级持久环境的上限（setx 的硬限制）。
const ENV_SECRET_MAX_VALUE: usize = 1024;

/// 判断一个环境变量名是否「疑似密钥」。
///
/// 命名启发式（全大写惯例下）：名字里带 `API_KEY` / `APIKEY`，或以
/// `_TOKEN` / `_SECRET` 结尾。`OPENAI_API_KEY`、`DEEPSEEK_API_KEY`、
/// `GITHUB_TOKEN` 这类都会命中；`PATH`、`HOME` 不会。
fn is_secret_candidate(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("API_KEY") || upper.contains("APIKEY") || upper.ends_with("_TOKEN") || upper.ends_with("_SECRET")
}

/// 把密钥值打成掩码预览：长值保留首尾各 4 字符，短值整体打码。
///
/// 后端对密钥的唯一展示形态 —— 任何命令都不向前端返回完整值。
fn mask_secret(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let n = chars.len();
    if n == 0 {
        return String::new();
    }
    if n <= 8 {
        return "*".repeat(n);
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[n - 4..].iter().collect();
    format!("{}…{}", head, tail)
}

/// 校验环境变量名（写入/删除前调用）：字母或下划线开头，只含字母数字下划线。
fn validate_env_name(name: &str) -> Result<(), CommandError> {
    let valid = !name.is_empty()
        && name.len() <= 253
        && name.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(CommandError::invalid_input(format!(
            "环境变量名「{name}」不合法：须以字母或下划线开头，只含字母、数字、下划线。"
        )))
    }
}

/// 枚举 Windows 用户级持久环境变量的名字（HKCU\Environment）。
///
/// 用于区分「本进程恰好有」与「已持久化」：进程环境可能来自启动本应用
/// 的父进程，未必写进了注册表。非 Windows 返回空集合（持久化判定仅在
/// Windows 提供，其他平台一律视为未持久化）。
#[cfg(windows)]
fn user_env_names() -> Vec<String> {
    let output = spawn_windows_tool("reg", &["query", r"HKCU\Environment"]);
    let Ok(output) = output else { return Vec::new() };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut names = Vec::new();
    for line in text.lines() {
        // 输出行形如 "    NAME    REG_SZ    value"（值可能为空、类型可能 REG_EXPAND_SZ）
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("HKEY_") {
            continue;
        }
        if let Some(name) = trimmed.split_whitespace().next() {
            if !name.is_empty() {
                names.push(name.to_string());
            }
        }
    }
    names
}

#[cfg(not(windows))]
fn user_env_names() -> Vec<String> {
    Vec::new()
}

/// 以 CREATE_NO_WINDOW 运行一个 Windows 自带工具（reg / setx），
/// 避免闪出控制台窗口。非 Windows 平台没有调用方。
#[cfg(windows)]
fn spawn_windows_tool(program: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(program)
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
}

/// 非 Windows 的占位：调用方都在 `if cfg!(windows)` 分支里，永不触达；
/// 保留签名让代码在所有平台都能编译。
#[cfg(not(windows))]
fn spawn_windows_tool(_program: &str, _args: &[&str]) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Windows-only"))
}

/// `env_secret_list`：枚举本机疑似密钥的环境变量（掩码展示）。
///
/// 只报告当前进程可见的变量（用户级 + 系统级 + 启动时继承）；
/// `persisted` 标记它是否同时存在于用户级持久环境。
/// 同步命令（Tauri 在工作线程池执行）：内部有一次注册表查询的阻塞调用。
#[tauri::command]
pub async fn env_secret_list() -> Result<Vec<EnvSecretInfo>, CommandError> {
    // 同步命令在 Tauri 里跑在主线程,而 reg query / setx 都可能因
    // WM_SETTINGCHANGE 广播阻塞数秒 —— 一律丢进阻塞线程池,不让 UI 卡住。
    tauri::async_runtime::spawn_blocking(|| {
        let persisted_names: std::collections::HashSet<String> =
            user_env_names().into_iter().collect();

        let mut items: Vec<EnvSecretInfo> = std::env::vars()
            .filter(|(name, value)| is_secret_candidate(name) && !value.is_empty())
            .map(|(name, value)| {
                let length = value.chars().count();
                EnvSecretInfo {
                    persisted: persisted_names.contains(&name),
                    name,
                    preview: mask_secret(&value),
                    length,
                }
            })
            .collect();

        items.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
        Ok(items)
    })
    .await
    .map_err(|e| CommandError::internal(format!("环境变量清单任务失败：{e}")))?
}

/// `env_secret_set`：写入/覆盖一个环境变量密钥。
///
/// 双写：`std::env::set_var` 让**本应用**（探测、连通测试走后端进程）
/// 立即可用；Windows 再用 `setx` 写用户级持久环境，**新开的**程序
/// （opencode / dsh / 终端）重启后才能读到 —— 注意正在运行的程序不会
/// 热更新。值超过 setx 的 1024 字符限制时仅本进程生效并在 note 说明。
#[tauri::command]
pub async fn env_secret_set(name: String, value: String) -> Result<EnvSecretWriteResult, CommandError> {
    validate_env_name(&name)?;
    if value.is_empty() {
        return Err(CommandError::invalid_input("密钥值不能为空。"));
    }

    tauri::async_runtime::spawn_blocking(move || {
        std::env::set_var(&name, &value);

        let mut result = EnvSecretWriteResult {
            process_set: true,
            persisted: false,
            note: String::new(),
        };

        if cfg!(windows) {
            if value.chars().count() > ENV_SECRET_MAX_VALUE {
                result.note = format!(
                    "值超过 {ENV_SECRET_MAX_VALUE} 字符（setx 上限），仅本应用内生效，未写入系统持久环境。"
                );
                return Ok(result);
            }
            match spawn_windows_tool("setx", &[&name, &value]) {
                Ok(out) if out.status.success() => result.persisted = true,
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    result.note = format!("本应用内已生效；写入系统持久环境失败：{}", stderr.trim());
                }
                Err(err) => {
                    result.note = format!("本应用内已生效；写入系统持久环境失败：{err}");
                }
            }
        } else {
            result.note = "本应用内已生效；当前平台不写 shell 配置，其他程序请自行 export。".to_string();
        }

        eprintln!("[ModelScout] 环境变量已设置：{}（持久化：{}）", name, result.persisted);
        Ok(result)
    })
    .await
    .map_err(|e| CommandError::internal(format!("环境变量写入任务失败：{e}")))?
}

/// `env_secret_delete`：删除一个环境变量密钥。
///
/// 同时清掉本进程变量与用户级持久环境（Windows 注册表）；删除不校验
/// 是否存在 —— 幂等。注意引用它的配置（`{env:NAME}`）会失效，界面层负责确认。
#[tauri::command]
pub async fn env_secret_delete(name: String) -> Result<(), CommandError> {
    validate_env_name(&name)?;
    tauri::async_runtime::spawn_blocking(move || {
        std::env::remove_var(&name);

        if cfg!(windows) {
            let _ = spawn_windows_tool("reg", &["delete", r"HKCU\Environment", "/v", &name, "/f"]);
        }
        eprintln!("[ModelScout] 环境变量已删除：{}", name);
    })
    .await
    .map_err(|e| CommandError::internal(format!("环境变量删除任务失败：{e}")))
}

/// `env_secret_status`：查询单个环境变量的精确存在性（掩码展示）。
///
/// 「疑似密钥清单」按命名启发式过滤，配置里引用的变量（如 `MY_KEY`）
/// 不一定在清单里 —— 判断「机器上到底有没有」以本命令为准。
#[tauri::command]
pub async fn env_secret_status(name: String) -> Result<EnvSecretStatus, CommandError> {
    validate_env_name(&name)?;
    tauri::async_runtime::spawn_blocking(move || {
        let persisted = user_env_names().iter().any(|n| n == &name);
        match std::env::var(&name) {
            Ok(value) if !value.is_empty() => Ok(EnvSecretStatus {
                exists: true,
                persisted,
                preview: mask_secret(&value),
                length: value.chars().count() as u64,
            }),
            _ => Ok(EnvSecretStatus {
                exists: false,
                persisted,
                preview: String::new(),
                length: 0,
            }),
        }
    })
    .await
    .map_err(|e| CommandError::internal(format!("环境变量状态任务失败：{e}")))?
}

// ---- 单元测试（仅覆盖不依赖 Tauri 运行时的纯逻辑） ----

#[cfg(test)]
mod tests {
    use super::{default_dsh_patch_path, default_opencode_config_path, expand_tilde, ping_failure};
    use crate::models::{ApiKeySource, ProbeConfig};
    use std::path::{Path, PathBuf};

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

    // ---- 一键填写：路径解析 ----

    #[test]
    fn tilde_expansion_rules() {
        let home = Path::new("/home/tester");
        assert_eq!(expand_tilde("~", home), PathBuf::from("/home/tester"));
        assert_eq!(expand_tilde("~/.config", home), PathBuf::from("/home/tester/.config"));
        assert_eq!(expand_tilde("~\\x", home), PathBuf::from("/home/tester/x"));
        assert_eq!(expand_tilde(" C:/abs/path ", home), PathBuf::from("C:/abs/path"));
    }

    #[test]
    fn opencode_default_path_falls_back_to_json() {
        // XDG_CONFIG_HOME 会影响结果，这里固定住再断言相对结构
        let tmp = std::env::temp_dir().join("modelscout-test-opencode-default");
        std::fs::remove_dir_all(&tmp).ok();
        let dir = tmp.join("opencode");
        std::fs::create_dir_all(&dir).ok();
        let _guard = xdg_guard(&tmp);

        let path = default_opencode_config_path(&tmp);
        assert_eq!(path, dir.join("opencode.json"));

        // 已有 opencode.jsonc 而无 opencode.json 时跟随 jsonc
        std::fs::write(dir.join("opencode.jsonc"), "{}").ok();
        let path = default_opencode_config_path(&tmp);
        assert_eq!(path, dir.join("opencode.jsonc"));

        std::fs::write(dir.join("opencode.json"), "{}").ok();
        let path = default_opencode_config_path(&tmp);
        assert_eq!(path, dir.join("opencode.json"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn dsh_default_path_prefers_web_profile_then_alpha() {
        let tmp = std::env::temp_dir().join("modelscout-test-dsh-default");
        std::fs::remove_dir_all(&tmp).ok();
        let _guard = dsh_home_guard(&tmp);

        // 没有任何 profile → web 兜底
        let path = default_dsh_patch_path(&tmp);
        assert_eq!(path, tmp.join("profiles").join("web").join("cordis.patch.yml"));

        // 有 alpha 和 web 两个 profile → web 优先
        for name in ["alpha", "web"] {
            let dir = tmp.join("profiles").join(name);
            std::fs::create_dir_all(&dir).ok();
            std::fs::write(dir.join("cordis.patch.yml"), "[]").ok();
        }
        let path = default_dsh_patch_path(&tmp);
        assert!(path.ends_with("profiles\\web\\cordis.patch.yml") || path.ends_with("profiles/web/cordis.patch.yml"));

        // 去掉 web，只剩 alpha → 字母序第一个
        std::fs::remove_file(tmp.join("profiles").join("web").join("cordis.patch.yml")).ok();
        let path = default_dsh_patch_path(&tmp);
        assert!(path.ends_with("profiles\\alpha\\cordis.patch.yml") || path.ends_with("profiles/alpha/cordis.patch.yml"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// 临时设置 XDG_CONFIG_HOME 并保证测试结束后还原（环境变量是进程级全局）。
    fn xdg_guard(dir: &Path) -> EnvRestore {
        EnvRestore::set("XDG_CONFIG_HOME", dir)
    }

    /// 临时把 DSH_HOME 指到临时目录。
    fn dsh_home_guard(dir: &Path) -> EnvRestore {
        EnvRestore::set("DSH_HOME", dir)
    }

    struct EnvRestore {
        key: &'static str,
        old: Option<std::ffi::OsString>,
    }

    impl EnvRestore {
        fn set(key: &'static str, value: &Path) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

// ---- 环境变量密钥管理的纯逻辑单测 ----

#[cfg(test)]
mod env_secret_tests {
    use super::{is_secret_candidate, mask_secret, validate_env_name};

    #[test]
    fn secret_candidate_matches_api_key_names() {
        assert!(is_secret_candidate("OPENAI_API_KEY"));
        assert!(is_secret_candidate("deepseek_api_key"));
        assert!(is_secret_candidate("MY_APIKEY"));
        assert!(is_secret_candidate("GITHUB_TOKEN"));
        assert!(is_secret_candidate("AWS_SECRET"));
        assert!(is_secret_candidate("API_KEY"));
        assert!(!is_secret_candidate("PATH"));
        assert!(!is_secret_candidate("HOME"));
        assert!(!is_secret_candidate("EDITOR"));
        // _KEY 结尾但不是 API_KEY：不误报
        assert!(!is_secret_candidate("SSH_KEY"));
    }

    #[test]
    fn mask_keeps_only_head_and_tail_for_long_values() {
        assert_eq!(mask_secret("sk-1234567890abcdefgh"), "sk-1…efgh");
        assert_eq!(mask_secret("sk-ms-test-1234567890abcd"), "sk-m…abcd");
    }

    #[test]
    fn mask_fully_hides_short_values_and_never_reveals_full() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("short"), "*****");
        assert_eq!(mask_secret("12345678"), "********");
        // 9 字符：长值规则，首尾各 4（重叠也不可能露出全部 9 位）
        assert_eq!(mask_secret("123456789"), "1234…6789");
    }

    #[test]
    fn env_name_validation() {
        assert!(validate_env_name("OPENAI_API_KEY").is_ok());
        assert!(validate_env_name("_PRIVATE").is_ok());
        assert!(validate_env_name("a1_B").is_ok());
        assert!(validate_env_name("").is_err());
        assert!(validate_env_name("1ABC").is_err());
        assert!(validate_env_name("HAS SPACE").is_err());
        assert!(validate_env_name("DASH-BAD").is_err());
        // 命令注入类输入必须被拒
        assert!(validate_env_name("A;calc").is_err());
        assert!(validate_env_name("A&b").is_err());
    }
}
