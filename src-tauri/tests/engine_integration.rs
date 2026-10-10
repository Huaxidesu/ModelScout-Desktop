//! ============================================================================
//! ModelScout —— 端到端集成测试（引擎层）
//! ============================================================================
//!
//! 【为什么要这个文件】（见 docs/DECISIONS.md D-37）
//!
//! 曾经有一个严重 bug：`run_engine()` 把结果写进了 `state.results`，
//! 却**忘了写进返回值**，导致命令返回空数组、界面永远显示「0/0 暂无探测结果」，
//! 而日志里每个模型都显示「完成」。
//!
//! 那个 bug 逃过了全部 36 个单元测试 —— 因为它们都是**纯函数**测试
//! （解析、正则、脱敏…），没有任何一个覆盖 `run_engine` 的**结果收集路径**。
//! `cargo check` 也报不出来：变量被读取过，不是 unused。
//!
//! 所以这里必须做**真集成**：起一个 mock HTTP 服务，驱动完整的
//! `fetch_model_list` → `run_engine` 流程，然后断言结果条数。
//!
//! 【关键设计：事件端口（EventSink）】
//! `run_engine` 原本直接依赖 `AppHandle`，因此无法在没有 Tauri 应用的场景下测试。
//! 现在它依赖 `probe::EventSink` 端口：
//!   - 生产环境传 `TauriSink`（内部就是 `AppHandle::emit`）；
//!   - 这里传 `RecordingSink`，把事件收进 `Vec`。
//!
//! 这样做除了解耦，还规避了一个真实的环境坑：
//! 若测试使用 `tauri::test::mock_app()`，就必须启用 `tauri` 的 `test` feature，
//! 而那会把完整 GUI 运行时（comctl32 / user32 / dwmapi…）链进测试二进制，
//! 在缺少 Windows 清单的 test 目标上会以
//! `STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139)` 启动失败（详见 D-37）。
//! ============================================================================

use std::sync::{Arc, Mutex};

use modelscout_lib::models::{ProbeConfig, ProbeLog, ProbeModelDone, ProbeProgress, ProbeStatus};
use modelscout_lib::probe::{
    fetch_model_list, run_engine, ApiClient, EngineOptions, EventSink, RunState,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// mock 服务返回的模型数量。三个模型分别覆盖两条路径：
///   - `plain-alpha` / `plain-beta`：/models 里**没有**扩展字段 → 走完整实测探测
///   - `rich-gamma`：/models 里**带**扩展 schema → 走「接口声明」路径
const MODEL_COUNT: usize = 3;

// ---------------------------------------------------------------------------
// 记录型事件端口
// ---------------------------------------------------------------------------

/// 把引擎发出的事件收进内存，便于断言。
#[derive(Clone, Default)]
struct RecordingSink {
    progress: Arc<Mutex<Vec<ProbeProgress>>>,
    logs: Arc<Mutex<Vec<ProbeLog>>>,
    model_done: Arc<Mutex<Vec<ProbeModelDone>>>,
}

impl RecordingSink {
    fn model_done_count(&self) -> usize {
        self.model_done.lock().expect("锁中毒").len()
    }
    fn log_count(&self) -> usize {
        self.logs.lock().expect("锁中毒").len()
    }
}

impl EventSink for RecordingSink {
    fn progress(&self, payload: ProbeProgress) {
        self.progress.lock().expect("锁中毒").push(payload);
    }
    fn log(&self, payload: ProbeLog) {
        self.logs.lock().expect("锁中毒").push(payload);
    }
    fn model_done(&self, payload: ProbeModelDone) {
        self.model_done.lock().expect("锁中毒").push(payload);
    }
}

// ---------------------------------------------------------------------------
// mock HTTP 服务
// ---------------------------------------------------------------------------

/// `/models` 响应：两个标准 OpenAI 条目 + 一个带扩展 schema 的条目。
fn models_body() -> String {
    r#"{
      "object": "list",
      "data": [
        { "id": "plain-alpha", "object": "model", "owned_by": "mock", "created": 1700000000 },
        { "id": "plain-beta",  "object": "model", "owned_by": "mock", "created": 1700000001 },
        {
          "schema_version": "2.4",
          "id": "rich-gamma",
          "name": "Rich Gamma",
          "created": 1700000002,
          "is_ready": true,
          "input_modalities": [
            { "type": "text",
              "supported_inputs": { "max_context_length": { "value": 262144, "unit": "token" } } },
            { "type": "image",
              "supported_inputs": { "sources": { "type": "enum", "values": ["url", "base64"] } } }
          ],
          "output_modalities": [
            { "type": "text",
              "max_length": { "value": 65536, "unit": "token" },
              "supported_parameters": {
                "max_tokens": { "type": "integer", "min": 1, "max": 65536 },
                "reasoning": { "type": "boolean" },
                "tools": { "type": "boolean" }
              } }
          ],
          "object": "model",
          "owned_by": "mock"
        }
      ]
    }"#
    .to_string()
}

/// `/chat/completions` 响应：带 `reasoning_content`，用于触发推理判定。
fn chat_body() -> String {
    r#"{
      "id": "chatcmpl-mock",
      "object": "chat.completion",
      "created": 1700000000,
      "model": "mock",
      "choices": [
        { "index": 0,
          "message": { "role": "assistant", "content": "hi", "reasoning_content": "thinking" },
          "finish_reason": "stop" }
      ]
    }"#
    .to_string()
}

/// 在字节切片里找子序列的起始下标。
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 处理单个连接：读请求 → 按路径返回固定响应 → 关闭。
async fn handle_conn(mut sock: tokio::net::TcpStream) {
    let mut buf = vec![0u8; 16 * 1024];
    let mut total = 0usize;

    // 1) 一直读到请求头结束（\r\n\r\n）
    let header_end = loop {
        match sock.read(&mut buf[total..]).await {
            Ok(0) | Err(_) => return,
            Ok(n) => total += n,
        }
        if let Some(pos) = find_subsequence(&buf[..total], b"\r\n\r\n") {
            break pos + 4;
        }
        if total >= buf.len() {
            return;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();

    // 2) 把请求体读干净（否则客户端可能因连接提前关闭而报错）
    let content_length = head
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body_read = total.saturating_sub(header_end);
    while body_read < content_length {
        match sock.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => body_read += n,
        }
    }

    // 3) 按路径返回
    let (status, body) = if path.ends_with("/models") {
        ("200 OK", models_body())
    } else if path.ends_with("/chat/completions") {
        ("200 OK", chat_body())
    } else {
        (
            "404 Not Found",
            r#"{"error":{"message":"not found"}}"#.to_string(),
        )
    };

    let resp = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    let _ = sock.write_all(resp.as_bytes()).await;
    let _ = sock.flush().await;
    let _ = sock.shutdown().await;
}

/// 起一个 mock HTTP 服务，返回它的 base URL（形如 `http://127.0.0.1:PORT/v1`）。
async fn start_mock_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock 服务绑定失败");
    let port = listener.local_addr().expect("取端口失败").port();

    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(handle_conn(sock));
        }
    });

    format!("http://127.0.0.1:{port}/v1")
}

/// 构造一个指向 mock 服务的 ApiClient。
fn mock_client(base_url: &str) -> ApiClient {
    let config = ProbeConfig {
        base_url: base_url.to_string(),
        api_key: "sk-mock-000000000000".to_string(),
        concurrency: 3,
        timeout_secs: 10,
        probe_limits: true,
        only_models: Vec::new(),
    };
    ApiClient::new(&config).expect("ApiClient 构造失败")
}

// ---------------------------------------------------------------------------
// 测试 1：D-37 的回归测试 —— 结果必须被完整收集
// ---------------------------------------------------------------------------

/// **这是 D-37 的回归测试。**
///
/// 断言 `run_engine` 的返回值里带着**全部**模型。
/// 修复前结果收集漏写，这里会得到 `0`，测试立即失败。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_engine_returns_every_probed_model() {
    let base = start_mock_server().await;
    let client = mock_client(&base);

    // 先拉列表（顺带覆盖 /models 的解析路径）
    let (briefs, _elapsed) = fetch_model_list(&client).await.expect("拉取模型列表失败");
    assert_eq!(
        briefs.len(),
        MODEL_COUNT,
        "mock 服务应返回 {MODEL_COUNT} 个模型"
    );

    let sink = RecordingSink::default();
    let state = RunState::new("run-integration".to_string(), briefs.len());
    let outcome = run_engine(
        sink.clone(),
        state.clone(),
        client,
        briefs,
        3,
        EngineOptions { probe_limits: true },
    )
    .await;

    // ★ 核心断言：一个模型都不能少。
    //   修复前这里会是 0（结果收集漏写），status 也会是 Failed。
    assert_eq!(
        outcome.models.len(),
        MODEL_COUNT,
        "run_engine 必须返回全部 {MODEL_COUNT} 个模型（漏收集会得到 0 —— 见 D-37）"
    );
    assert_eq!(
        outcome.status,
        ProbeStatus::Done,
        "有结果时状态应为 Done，而非 Failed"
    );

    // 结果应按 id 排序，保证前端表格顺序稳定
    let ids: Vec<&str> = outcome.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["plain-alpha", "plain-beta", "rich-gamma"]);

    // state.results 与返回值应一致（两者是同一份数据的两条出口）
    let cached = state.results.lock().await;
    assert_eq!(
        cached.len(),
        MODEL_COUNT,
        "state.results 也必须记录全部结果（供快照/重同步使用）"
    );
    drop(cached);

    // 事件端口应收到 N 条 model-done 与若干日志
    assert_eq!(
        sink.model_done_count(),
        MODEL_COUNT,
        "每个模型完成时都应发出一次 model-done 事件"
    );
    assert!(sink.log_count() > 0, "应有日志事件");
}

// ---------------------------------------------------------------------------
// 测试 2：接口声明路径 —— 扩展 schema 的值应被直接采信
// ---------------------------------------------------------------------------

/// 带扩展 schema 的模型，其上下文/输出/工具/视觉/推理应**全部来自接口声明**，
/// 且来源标记为「接口声明」；标准 OpenAI 条目则走实测路径。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_capabilities_are_trusted_without_probing() {
    let base = start_mock_server().await;
    let client = mock_client(&base);
    let (briefs, _) = fetch_model_list(&client).await.expect("拉取列表失败");

    let outcome = run_engine(
        RecordingSink::default(),
        RunState::new("run-declared".to_string(), briefs.len()),
        client,
        briefs,
        3,
        EngineOptions { probe_limits: true },
    )
    .await;

    let gamma = outcome
        .models
        .iter()
        .find(|m| m.id == "rich-gamma")
        .expect("结果中应包含 rich-gamma");

    assert_eq!(gamma.context_limit, 262144, "上下文应取自接口声明");
    assert_eq!(gamma.max_output_limit, 65536, "输出上限应取自接口声明");
    assert!(gamma.is_reasoning, "reasoning 参数存在 → 推理模型");
    assert!(
        gamma.caps_source.starts_with("接口声明"),
        "来源应标为接口声明，实际：{}",
        gamma.caps_source
    );

    // 未声明的模型应走实测路径
    let alpha = outcome
        .models
        .iter()
        .find(|m| m.id == "plain-alpha")
        .expect("结果中应包含 plain-alpha");
    assert_eq!(
        alpha.caps_source, "实测探测",
        "标准 OpenAI 条目应走实测探测路径"
    );
}

// ---------------------------------------------------------------------------
// 测试 3：中止路径 —— 不应 panic，结果数不超过模型总数
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_run_keeps_finished_models() {
    let base = start_mock_server().await;
    let client = mock_client(&base);
    let (briefs, _) = fetch_model_list(&client).await.expect("拉取列表失败");

    let state = RunState::new("run-cancel".to_string(), briefs.len());
    // 立刻置中止标志：循环会在消费完已派发的任务后 break
    state.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);

    let outcome = run_engine(
        RecordingSink::default(),
        state,
        client,
        briefs,
        3,
        EngineOptions { probe_limits: true },
    )
    .await;

    assert_eq!(outcome.status, ProbeStatus::Cancelled, "应识别为已取消");
    assert!(
        outcome.models.len() <= MODEL_COUNT,
        "中止时结果数不应超过模型总数"
    );
}

// ---------------------------------------------------------------------------
// 测试 4：空列表 —— 应返回 Failed 且不 panic
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_model_list_yields_failed_without_panic() {
    let base = start_mock_server().await;
    let client = mock_client(&base);

    let outcome = run_engine(
        RecordingSink::default(),
        RunState::new("run-empty".to_string(), 0),
        client,
        Vec::new(),
        3,
        EngineOptions { probe_limits: true },
    )
    .await;

    assert_eq!(outcome.status, ProbeStatus::Failed);
    assert!(outcome.models.is_empty());
    assert!(
        !outcome.errors.is_empty(),
        "应给出「模型列表为空」的说明性错误"
    );
}
