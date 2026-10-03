use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::{
    Router,
    body::Bytes,
    extract::{Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::{Stream, StreamExt};
use jni::objects::GlobalRef;
use serde::Deserialize;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single SSE event (event-type + data line).
#[derive(Clone)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
}

/// Tool callback stored once per server after Java calls registerToolCallback.
pub struct ToolCallbackRef {
    pub java_vm: Arc<jni::JavaVM>,
    /// Arc so cloning in mcp.rs never needs JVM thread-attachment (no NewGlobalRef/DeleteGlobalRef
    /// on tokio async threads); only the original GlobalRef creation and its final drop touch JNI.
    pub callback: Arc<GlobalRef>,
}

/// Status callback stored once per server after Java calls registerStatusCallback.
/// Deliberately a sibling of `ToolCallbackRef` (not a reuse): the status-line channel
/// is independent of the MCP tool path, so it gets its own ref, its own Java method
/// (`onStatusUpdate`), and its own invoker (`call_java_status`).
pub struct StatusCallbackRef {
    pub java_vm: Arc<jni::JavaVM>,
    pub callback: Arc<GlobalRef>,
}

// ---------------------------------------------------------------------------
// Shared state (Arc'd into every Axum handler)
// ---------------------------------------------------------------------------

pub struct AppState {
    /// Keyed by sessionId → unbounded sender for that SSE stream.
    pub clients: Mutex<HashMap<String, mpsc::UnboundedSender<SseEvent>>>,
    pub auth_token: String,
    pub tool_callback: Mutex<Option<ToolCallbackRef>>,
    /// Dedicated status-line callback, separate from `tool_callback` (status is not a tool).
    pub status_callback: Mutex<Option<StatusCallbackRef>>,
    pub preferred_port: Option<u16>,
    /// Last "selection_changed" notification JSON, cached so a CLI that connects
    /// after a selection happened can be replayed it on initialize.
    pub last_selection: Mutex<Option<String>>,
    /// The same notification with its lines counted from 0, for the sessions in
    /// `sessions_counting_from_zero` (see `selection_message`).
    pub last_selection_from_zero: Mutex<Option<String>>,
    /// Sessions whose client announced itself as a CLI that counts selection lines
    /// from 0. Every other session is sent lines as Eclipse labels them.
    pub sessions_counting_from_zero: Mutex<HashSet<String>>,
}

// ---------------------------------------------------------------------------
// Selection data (passed from Java on every selection change)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SelectionArgs {
    file_path: String,
    text: String,
    start_line: i32,
    end_line: i32,
    start_col: i32,
    end_col: i32,
    is_empty: bool,
}

/// The "selection_changed" notification the CLI reads live editor context from:
/// snake_case, shaped `{ selection:{start,end}, text, filePath }`. Lines arrive from
/// Java as Eclipse labels them (from 1); columns are real 0-based offsets in the line.
///
/// The CLI's consumer has read the lines two ways (both checked in the binaries):
///     lineCount = end.line - start.line + 1;
///     if (end.character === 0) lineCount--;     // ended at a line start
///     lineStart = start.line;                    // up to 2.1.179: shown as given
///     lineStart = start.line + 1;                // 2.1.181 on: counted from 0
///     lineEnd   = lineStart + lineCount - 1;
/// So `from_zero` moves both lines up one for a CLI that adds it back; every other
/// client gets them as given. Columns must stay real: a hardcoded end character 0
/// makes the `=== 0` rule drop the last selected line.
/// A bare caret (no highlighted text) is sent as a null range so Claude still
/// learns the active file via filePath.
fn selection_message(args: &SelectionArgs, from_zero: bool) -> String {
    let line = |n: i32| if from_zero { (n - 1).max(0) } else { n };
    let selection = if args.is_empty {
        serde_json::Value::Null
    } else {
        serde_json::json!({
            "start": { "line": line(args.start_line), "character": args.start_col },
            "end":   { "line": line(args.end_line),   "character": args.end_col }
        })
    };
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "selection_changed",
        "params": {
            "selection": selection,
            "text": args.text,
            "filePath": args.file_path
        }
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Server  (one instance per plugin lifecycle)
// ---------------------------------------------------------------------------

pub struct Server {
    // NOTE: Field declaration order = drop order.
    // `runtime` must drop before `state` so that all tokio tasks (which hold
    // Arc<AppState> clones) complete and release their references before the
    // Server's own Arc<AppState> clone is decremented.  Runtime::drop() blocks
    // until all tasks finish, guaranteeing AppState outlives every task.
    runtime: Runtime,
    pub state: Arc<AppState>,
    port_min: u16,
    port_max: u16,
    shutdown_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Handle to the Axum serve task. `axum::serve` owns the TcpListener and only
    /// releases it when its future returns, which graceful shutdown will not do
    /// while any client still holds a connection open. Keeping the handle lets
    /// stop() cancel the task outright so the port is free before we return.
    serve_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    port: AtomicU16,
    running: AtomicBool,
    /// Pending debounce task for selection-changed notifications (50 ms).
    /// Aborted in stop() before runtime shuts down.
    selection_debounce: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Server {
    pub fn new(port_min: u16, port_max: u16) -> Self {
        Self::new_with_config(port_min, port_max, None, None)
    }

    pub fn new_with_config(
        port_min: u16,
        port_max: u16,
        preferred_port: Option<u16>,
        existing_token: Option<String>,
    ) -> Self {
        let auth_token = existing_token.unwrap_or_else(|| Uuid::new_v4().to_string());
        let state = Arc::new(AppState {
            clients: Mutex::new(HashMap::new()),
            auth_token,
            tool_callback: Mutex::new(None),
            status_callback: Mutex::new(None),
            preferred_port,
            last_selection: Mutex::new(None),
            last_selection_from_zero: Mutex::new(None),
            sessions_counting_from_zero: Mutex::new(HashSet::new()),
        });
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("claude-eclipse")
            .build()
            .expect("Failed to build tokio runtime");

        Server {
            runtime,
            state,
            port_min,
            port_max,
            shutdown_tx: Mutex::new(None),
            serve_task: Mutex::new(None),
            port: AtomicU16::new(0),
            running: AtomicBool::new(false),
            selection_debounce: Mutex::new(None),
        }
    }

    /// Binds the first available port and spawns the Axum server.  Returns the port.
    pub fn start(&self) -> u16 {
        if self.running.load(Ordering::Relaxed) {
            return self.port.load(Ordering::Relaxed);
        }

        // Bind synchronously before spawning so we can return the port number.
        let (port_min, port_max) = (self.port_min, self.port_max);
        let preferred = self.state.preferred_port;
        let listener = self
            .runtime
            .block_on(async move {
                // Try preferred port first if specified
                if let Some(pref) = preferred {
                    if let Ok(l) = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", pref)).await {
                        return Ok(l);
                    }
                }
                // Fall back to scanning the range
                for p in port_min..=port_max {
                    if let Ok(l) =
                        tokio::net::TcpListener::bind(format!("127.0.0.1:{}", p)).await
                    {
                        return Ok(l);
                    }
                }
                Err(std::io::Error::other("No available port"))
            });

        // No bindable port in the configured range — an empty range, or one whose
        // every port is already taken. Report failure instead of panicking: this is
        // called through JNI, where an unwind across the extern "system" boundary
        // aborts the whole IDE. Java's HttpSseServer.start() already treats 0 as
        // "did not start" and leaves `running` false.
        let listener = match listener {
            Ok(l) => l,
            Err(_) => {
                if crate::is_debug() {
                    eprintln!(
                        "[server] no free port in range {}-{}; server not started",
                        port_min, port_max
                    );
                }
                return 0;
            }
        };

        let bound_port = listener.local_addr().unwrap().port();
        self.port.store(bound_port, Ordering::Relaxed);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        *self.shutdown_tx.lock().unwrap() = Some(shutdown_tx);

        let state = Arc::clone(&self.state);
        let serve_task = self.runtime.spawn(async move {
            let app = Router::new()
                .route("/sse", get(sse_handler))
                .route("/messages", post(messages_handler))
                .route("/statusline", post(statusline_handler))
                .with_state(state);

            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });
        *self.serve_task.lock().unwrap() = Some(serve_task);

        self.running.store(true, Ordering::Relaxed);
        bound_port
    }

    fn stop(&self) {
        if self
            .running
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::Relaxed)
            .is_err()
        {
            return; // already stopped
        }

        // Abort any pending selection debounce task.
        if let Some(handle) = self.selection_debounce.lock().unwrap().take() {
            handle.abort();
        }

        // Drop all SSE senders first so their streams return Poll::Ready(None)
        // and the Axum tasks complete before the runtime shuts down.
        self.state.clients.lock().unwrap().clear();
        self.state.sessions_counting_from_zero.lock().unwrap().clear();

        // Signal Axum to stop accepting new connections, giving in-flight responses
        // the chance to finish on their own.
        if let Some(tx) = self.shutdown_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }

        // Then cancel the serve task and wait for the cancellation to land, so the
        // TcpListener is dropped and the port is free before this returns.
        //
        // Graceful shutdown alone is not enough: it waits for every open connection,
        // and a connected CLI keeps one open indefinitely, so the listener stayed
        // bound and the next start had to pick a different port (measured: a server
        // reused its port across 13 restarts with no conversation open, then climbed
        // the moment one connected). Every caller that restarts this server also
        // reconnects its CLI sessions, so cutting the old connections here tears
        // down nothing that was not already being replaced.
        //
        // block_on is safe: stop() is only reached from the JNI thread (serverStop
        // or Drop), never from inside the runtime.
        if let Some(task) = self.serve_task.lock().unwrap().take() {
            task.abort();
            let _ = self.runtime.block_on(task);
        }
    }

    /// Called from Java on every raw selection event.
    /// Debounces 50 ms, caches the latest state, then broadcasts a "selection_changed"
    /// message to all connected SSE clients.
    pub fn notify_selection(
        &self,
        file_path: String,
        text: String,
        start_line: i32,
        end_line: i32,
        start_col: i32,
        end_col: i32,
        is_empty: bool,
    ) {
        if !self.running.load(Ordering::Relaxed) {
            return;
        }

        // Cancel any previous pending broadcast.
        if let Some(handle) = self.selection_debounce.lock().unwrap().take() {
            handle.abort();
        }

        let args = SelectionArgs { file_path, text, start_line, end_line, start_col, end_col, is_empty };
        let state = Arc::clone(&self.state);

        let join_handle = self.runtime.spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

            // One message per way the CLI reads lines; see selection_message.
            let as_given = selection_message(&args, false);
            let from_zero = selection_message(&args, true);

            // Cache for replay to clients that connect later (open file -> start
            // Claude). Stored even when no client is connected yet.
            *state.last_selection.lock().unwrap() = Some(as_given.clone());
            *state.last_selection_from_zero.lock().unwrap() = Some(from_zero.clone());

            // Broadcast to any currently-connected clients, each in its own numbering.
            let counting_from_zero = state.sessions_counting_from_zero.lock().unwrap().clone();
            let mut clients = state.clients.lock().unwrap();
            if clients.is_empty() {
                return;
            }
            clients.retain(|session, tx| {
                let data = if counting_from_zero.contains(session) { &from_zero } else { &as_given };
                tx.send(SseEvent { event_type: "message".to_string(), data: data.clone() }).is_ok()
            });
        });

        *self.selection_debounce.lock().unwrap() = Some(join_handle);
    }

    pub fn broadcast(&self, json: &str) {
        let event = SseEvent {
            event_type: "message".to_string(),
            data: json.to_string(),
        };
        let mut clients = self.state.clients.lock().unwrap();
        // retain() removes clients whose channel has been closed (disconnected).
        clients.retain(|_, tx| tx.send(event.clone()).is_ok());
    }

    pub fn client_count(&self) -> usize {
        let mut clients = self.state.clients.lock().unwrap();
        // Prune clients whose receiver has been dropped (TCP connection dead).
        clients.retain(|_, tx| !tx.is_closed());
        clients.len()
    }

    pub fn port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    pub fn auth_token(&self) -> &str {
        &self.state.auth_token
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn register_tool_callback(&self, vm: Arc<jni::JavaVM>, callback: GlobalRef) {
        *self.state.tool_callback.lock().unwrap() = Some(ToolCallbackRef {
            java_vm: vm,
            callback: Arc::new(callback),
        });
    }

    pub fn register_status_callback(&self, vm: Arc<jni::JavaVM>, callback: GlobalRef) {
        *self.state.status_callback.lock().unwrap() = Some(StatusCallbackRef {
            java_vm: vm,
            callback: Arc::new(callback),
        });
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
        // The Runtime is dropped after this, which blocks until all Tokio tasks finish.
        // Because stop() already closed all SSE senders, tasks complete quickly.
    }
}

// ---------------------------------------------------------------------------
// Axum handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionQuery {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

/// True only if the request targets a loopback `Host` and carries no foreign
/// `Origin`. This is the security boundary for the local MCP server: it blocks
/// DNS-rebinding / browser-based access (a malicious page rebinding its domain
/// to 127.0.0.1 sends `Host: attacker.tld`, which is rejected). The Claude CLI
/// sends `Host: 127.0.0.1:<port>` and no `Origin`, so real clients are unaffected.
fn is_loopback_host(value: &str) -> bool {
    // Strip a trailing :port, then compare the host part.
    let host = value.rsplit_once(':').map(|(h, _)| h).unwrap_or(value);
    host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost")
}

fn is_local_request(headers: &axum::http::HeaderMap) -> bool {
    let host_ok = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(is_loopback_host)
        .unwrap_or(false);

    // A non-browser client (the CLI) sends no Origin, which is allowed. If an
    // Origin IS present, its host must be loopback too.
    let origin_ok = match headers
        .get(axum::http::header::ORIGIN)
        .and_then(|h| h.to_str().ok())
    {
        None => true,
        Some(origin) => {
            let after_scheme = origin.split("://").nth(1).unwrap_or(origin);
            let host = after_scheme.split('/').next().unwrap_or(after_scheme);
            is_loopback_host(host)
        }
    };

    host_ok && origin_ok
}

/// GET /sse — establishes a long-lived Server-Sent Events stream.
async fn sse_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Reject anything that isn't a loopback-origin request (DNS-rebinding guard).
    if !is_local_request(&headers) {
        if crate::is_debug() {
            eprintln!("[sse_handler] rejected non-local request (Host/Origin not loopback)");
        }
        return StatusCode::FORBIDDEN.into_response();
    }

    let (tx, rx) = mpsc::unbounded_channel::<SseEvent>();
    let session_id = Uuid::new_v4().to_string();

    // Tell the client where to POST messages.
    let endpoint_url = format!("/messages?sessionId={}", session_id);
    let _ = tx.send(SseEvent {
        event_type: "endpoint".to_string(),
        data: endpoint_url,
    });

    // Register sender; the GuardedStream below will remove it on disconnect.
    state.clients.lock().unwrap().insert(session_id.clone(), tx);

    let inner = UnboundedReceiverStream::new(rx).map(|ev: SseEvent| {
        Ok::<Event, Infallible>(Event::default().event(ev.event_type).data(ev.data))
    });

    let guarded = GuardedStream {
        inner,
        guard: Some(ClientGuard {
            session_id,
            state: Arc::clone(&state),
        }),
    };

    Sse::new(guarded)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(3))
                .text(":"),
        )
        .into_response()
}

/// POST /messages — receives a JSON-RPC 2.0 body and processes it asynchronously.
async fn messages_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SessionQuery>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    // Reject anything that isn't a loopback-origin request (DNS-rebinding guard).
    if !is_local_request(&headers) {
        if crate::is_debug() {
            eprintln!("[messages_handler] rejected non-local request (Host/Origin not loopback)");
        }
        return StatusCode::FORBIDDEN.into_response();
    }

    let session_id = match params.session_id {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, "Missing sessionId").into_response(),
    };

    let sender = {
        let clients = state.clients.lock().unwrap();
        clients.get(&session_id).cloned()
    };

    let sender = match sender {
        Some(s) => s,
        None => return (StatusCode::BAD_REQUEST, "Unknown session").into_response(),
    };

    let body_str = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8 body").into_response(),
    };

    // Respond 202 immediately; the actual JSON-RPC reply arrives over SSE.
    tokio::spawn(async move {
        crate::mcp::handle_message(state, session_id, sender, body_str).await;
    });

    (StatusCode::ACCEPTED, "Accepted").into_response()
}

// ---------------------------------------------------------------------------
// /statusline — dedicated status-line channel (NOT the MCP tool path)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StatusQuery {
    /// Our per-tab routing token (minted by ClaudeCliView, echoed by StandaloneStatusForwarder).
    tab: Option<String>,
    /// The workspace server's shared secret (lock-file `authToken` / `state.auth_token`).
    #[serde(rename = "authToken")]
    auth_token: Option<String>,
}

/// POST /statusline?tab=<tabToken>&authToken=<secret>
/// Receives the raw Claude statusLine JSON (piped to StandaloneStatusForwarder's stdin and
/// forwarded verbatim) and routes it to the matching tab via the status callback.
async fn statusline_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StatusQuery>,
    body: Bytes,
) -> Response {
    let tab = match params.tab {
        Some(t) if !t.is_empty() => t,
        _ => return (StatusCode::BAD_REQUEST, "Missing tab").into_response(),
    };

    // Authenticate with the workspace server's shared secret (loopback-only endpoint,
    // random per-session token).
    let presented = params.auth_token.unwrap_or_default();
    if presented != state.auth_token {
        return (StatusCode::UNAUTHORIZED, "Invalid authToken").into_response();
    }

    let body_str = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8 body").into_response(),
    };

    // Hand off to Java on a plain OS thread so attach_current_thread is safe even off a
    // tokio worker (mirrors ClientGuard::drop). The forwarder ignores the response body.
    let guard = state.status_callback.lock().unwrap();
    if let Some(cb) = guard.as_ref() {
        let java_vm  = Arc::clone(&cb.java_vm);
        let callback = Arc::clone(&cb.callback);
        std::thread::spawn(move || {
            call_java_status(&java_vm, &callback, &tab, &body_str);
        });
    }

    (StatusCode::OK, "OK").into_response()
}

/// Attaches the current thread to the JVM and calls
/// `callback.onStatusUpdate(tabToken, statusJson)`. Dedicated to the status channel —
/// deliberately separate from `mcp::call_java_tool`.
fn call_java_status(
    java_vm: &jni::JavaVM,
    callback: &GlobalRef,
    tab: &str,
    status_json: &str,
) {
    use jni::objects::{JObject, JValue};

    let mut env = match java_vm.attach_current_thread() {
        Ok(env) => env,
        Err(e) => {
            if crate::is_debug() {
                eprintln!("statusline: JVM attach failed: {}", e);
            }
            return;
        }
    };

    let tab_jstr = match env.new_string(tab) {
        Ok(s) => s,
        Err(_) => return,
    };
    let json_jstr = match env.new_string(status_json) {
        Ok(s) => s,
        Err(_) => return,
    };
    let tab_obj  = JObject::from(tab_jstr);
    let json_obj = JObject::from(json_jstr);

    let result = env.call_method(
        callback,
        "onStatusUpdate",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        &[JValue::Object(&tab_obj), JValue::Object(&json_obj)],
    );
    if result.is_err() {
        let _ = env.exception_clear();
    }
}

// ---------------------------------------------------------------------------
// GuardedStream — removes the SSE client from the map when it drops.
// ---------------------------------------------------------------------------

struct ClientGuard {
    session_id: String,
    state: Arc<AppState>,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.state.clients.lock().unwrap().remove(&self.session_id);
        self.state.sessions_counting_from_zero.lock().unwrap().remove(&self.session_id);

        // Close any pending Eclipse diff tabs when this MCP client disconnects.
        // Spawn a plain OS thread so JNI can safely attach — drop() is synchronous
        // and may not execute inside a Tokio task.
        let guard = self.state.tool_callback.lock().unwrap();
        if let Some(cb) = guard.as_ref() {
            let java_vm  = Arc::clone(&cb.java_vm);
            let callback = Arc::clone(&cb.callback);
            std::thread::spawn(move || {
                crate::mcp::call_java_tool(&java_vm, &callback, "closeAllDiffTabs", "{}");
            });
        }
    }
}

struct GuardedStream<S> {
    inner: S,
    guard: Option<ClientGuard>,
}

impl<S: Stream + Unpin> Stream for GuardedStream<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(None) => {
                // Stream exhausted — drop guard so cleanup runs immediately.
                self.guard.take();
                Poll::Ready(None)
            }
            other => other,
        }
    }
}

// Axum requires the SSE stream to be Send.
unsafe impl<S: Send> Send for GuardedStream<S> {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, HeaderMap, HeaderValue};
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn headers(host: Option<&str>, origin: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(v) = host {
            h.insert(header::HOST, HeaderValue::from_str(v).unwrap());
        }
        if let Some(v) = origin {
            h.insert(header::ORIGIN, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    /// A server on a port the OS just handed out, well away from the range the
    /// plugin's own MCP server uses. Nothing is registered, so no request reaches Java.
    fn start_server() -> (Server, u16) {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let server = Server::new(port, port);
        assert_eq!(server.start(), port, "the server binds the port it was given");
        (server, port)
    }

    fn connect(port: u16) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream
    }

    /// Sends one request with `Connection: close` and returns (status, whole response).
    fn request(port: u16, method: &str, path: &str, host: &str, extra: &str, body: &str) -> (u16, String) {
        let mut stream = connect(port);
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        let status = response.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        (status, response)
    }

    /// Reads from an open SSE stream until `needle` has arrived, and returns all of it.
    fn read_until(stream: &mut TcpStream, seen: &mut String, needle: &str) {
        let mut buf = [0u8; 4096];
        while !seen.contains(needle) {
            let n = stream.read(&mut buf).expect("the stream stays open and delivers");
            assert!(n > 0, "the server closed the stream before sending {needle:?}");
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }

    #[test]
    fn only_loopback_host_names_count_as_local() {
        for local in ["127.0.0.1", "127.0.0.1:48123", "localhost", "LOCALHOST:80"] {
            assert!(is_loopback_host(local), "{local}");
        }
        for foreign in ["evil.example", "evil.example:80", "127.0.0.2:80", "localhost.evil.example", "evil:127.0.0.1"] {
            assert!(!is_loopback_host(foreign), "{foreign}");
        }
    }

    #[test]
    fn a_request_is_local_only_with_a_loopback_host_and_no_foreign_origin() {
        assert!(is_local_request(&headers(Some("127.0.0.1:1"), None)), "the CLI sends no Origin");
        assert!(is_local_request(&headers(Some("127.0.0.1:1"), Some("http://localhost:3000"))));

        // DNS rebinding: the page's own name arrives as Host.
        assert!(!is_local_request(&headers(Some("evil.example:1"), None)));
        assert!(!is_local_request(&headers(Some("127.0.0.1:1"), Some("http://evil.example"))));
        assert!(!is_local_request(&headers(Some("127.0.0.1:1"), Some("null"))));
        assert!(!is_local_request(&headers(None, None)), "no Host at all");
    }

    #[test]
    fn a_foreign_host_or_origin_is_refused_on_both_mcp_routes() {
        let (_server, port) = start_server();
        let local = format!("127.0.0.1:{port}");
        assert_eq!(request(port, "GET", "/sse", "evil.example", "", "").0, 403);
        assert_eq!(request(port, "GET", "/sse", &local, "Origin: http://evil.example\r\n", "").0, 403);
        assert_eq!(request(port, "POST", "/messages?sessionId=x", "evil.example", "", "{}").0, 403);
    }

    #[test]
    fn messages_need_a_known_session() {
        let (_server, port) = start_server();
        let local = format!("127.0.0.1:{port}");
        let (status, body) = request(port, "POST", "/messages", &local, "", "{}");
        assert_eq!(status, 400);
        assert!(body.ends_with("Missing sessionId"), "{body}");
        let (status, body) = request(port, "POST", "/messages?sessionId=nobody", &local, "", "{}");
        assert_eq!(status, 400);
        assert!(body.ends_with("Unknown session"), "{body}");
    }

    #[test]
    fn the_status_line_needs_a_tab_and_the_servers_token() {
        let (server, port) = start_server();
        let local = format!("127.0.0.1:{port}");
        let token = server.auth_token().to_string();

        let (status, _) = request(port, "POST", &format!("/statusline?authToken={token}"), &local, "", "{}");
        assert_eq!(status, 400, "no tab");
        let (status, _) = request(port, "POST", "/statusline?tab=t1&authToken=wrong", &local, "", "{}");
        assert_eq!(status, 401);
        let (status, _) = request(port, "POST", &format!("/statusline?tab=t1&authToken={token}"), &local, "", "{}");
        assert_eq!(status, 200);
    }

    #[test]
    fn an_sse_client_gets_its_endpoint_and_the_answer_to_initialize() {
        let (server, port) = start_server();
        let local = format!("127.0.0.1:{port}");

        let mut sse = connect(port);
        write!(sse, "GET /sse HTTP/1.1\r\nHost: {local}\r\nAccept: text/event-stream\r\n\r\n").unwrap();
        let mut seen = String::new();
        read_until(&mut sse, &mut seen, "\n\n");
        read_until(&mut sse, &mut seen, "sessionId=");
        read_until(&mut sse, &mut seen, "\n");
        assert!(seen.starts_with("HTTP/1.1 200"), "{seen}");
        assert!(seen.contains("event: endpoint"), "{seen}");
        let endpoint = seen
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .expect("the endpoint event carries the POST path")
            .trim()
            .to_string();
        assert!(endpoint.starts_with("/messages?sessionId="), "{endpoint}");
        assert_eq!(server.client_count(), 1);

        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let (status, _) = request(port, "POST", &endpoint, &local, "", init);
        assert_eq!(status, 202, "accepted; the answer comes over SSE");

        read_until(&mut sse, &mut seen, "protocolVersion");
        read_until(&mut sse, &mut seen, "\n");
        assert!(seen.contains("event: message"), "{seen}");
        assert!(seen.contains(r#""protocolVersion":"2024-11-05""#), "{seen}");
    }

    fn selection(start_line: i32, end_line: i32, start_col: i32, end_col: i32) -> SelectionArgs {
        SelectionArgs {
            file_path: "C:/a.rs".to_string(),
            text: "x".to_string(),
            start_line,
            end_line,
            start_col,
            end_col,
            is_empty: false,
        }
    }

    #[test]
    fn selection_lines_as_given_are_sent_exactly_as_before() {
        assert_eq!(
            selection_message(&selection(22, 23, 4, 77), false),
            r#"{"jsonrpc":"2.0","method":"selection_changed","params":{"filePath":"C:/a.rs","selection":{"end":{"character":77,"line":23},"start":{"character":4,"line":22}},"text":"x"}}"#
        );
    }

    #[test]
    fn selection_lines_from_zero_move_both_lines_up_one_and_keep_the_columns() {
        let as_given: serde_json::Value = serde_json::from_str(&selection_message(&selection(22, 23, 4, 77), false)).unwrap();
        let from_zero: serde_json::Value = serde_json::from_str(&selection_message(&selection(22, 23, 4, 77), true)).unwrap();
        assert_eq!(from_zero["params"]["selection"]["start"], serde_json::json!({"line": 21, "character": 4}));
        assert_eq!(from_zero["params"]["selection"]["end"], serde_json::json!({"line": 22, "character": 77}));
        assert_eq!(from_zero["params"]["filePath"], as_given["params"]["filePath"]);
        assert_eq!(from_zero["params"]["text"], as_given["params"]["text"]);
        assert_eq!(from_zero["method"], "selection_changed");
    }

    #[test]
    fn a_line_from_zero_never_goes_below_zero() {
        let from_zero: serde_json::Value = serde_json::from_str(&selection_message(&selection(0, 0, 0, 1), true)).unwrap();
        assert_eq!(from_zero["params"]["selection"]["start"]["line"], 0);
        assert_eq!(from_zero["params"]["selection"]["end"]["line"], 0);
    }

    #[test]
    fn a_bare_caret_is_the_same_message_either_way() {
        let caret = SelectionArgs { is_empty: true, ..selection(22, 22, 5, 5) };
        assert_eq!(selection_message(&caret, false), selection_message(&caret, true));
        let msg: serde_json::Value = serde_json::from_str(&selection_message(&caret, false)).unwrap();
        assert!(msg["params"]["selection"].is_null(), "a caret is sent as a null range");
    }

    /// Opens an SSE stream, initializes it as a client announcing `client_info`, and
    /// returns the stream once the answer to initialize has arrived.
    fn initialized_client(port: u16, client_info: &str) -> (TcpStream, String) {
        let local = format!("127.0.0.1:{port}");
        let mut sse = connect(port);
        write!(sse, "GET /sse HTTP/1.1\r\nHost: {local}\r\nAccept: text/event-stream\r\n\r\n").unwrap();
        let mut seen = String::new();
        read_until(&mut sse, &mut seen, "sessionId=");
        read_until(&mut sse, &mut seen, "\n");
        let endpoint = seen.lines().find_map(|l| l.strip_prefix("data: ")).unwrap().trim().to_string();
        let init = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"clientInfo":{client_info}}}}}"#);
        assert_eq!(request(port, "POST", &endpoint, &local, "", &init).0, 202);
        read_until(&mut sse, &mut seen, "protocolVersion");
        (sse, seen)
    }

    /// The first selection_changed message on an SSE stream.
    fn next_selection(sse: &mut TcpStream, seen: &mut String) -> serde_json::Value {
        read_until(sse, seen, "selection_changed");
        let at = seen.find("selection_changed").unwrap();
        let mut buf = [0u8; 4096];
        while !seen[at..].contains('\n') {
            let n = sse.read(&mut buf).expect("the stream stays open and delivers");
            assert!(n > 0, "the server closed the stream mid-message");
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        let line = seen.lines().find(|l| l.contains("selection_changed")).unwrap();
        serde_json::from_str(line.strip_prefix("data: ").unwrap()).unwrap()
    }

    #[test]
    fn each_client_gets_the_selection_in_the_line_numbering_it_reads() {
        let (server, port) = start_server();
        let (mut new_cli, mut new_seen) = initialized_client(port, r#"{"name":"claude-code","version":"2.1.287"}"#);
        let (mut old_cli, mut old_seen) = initialized_client(port, r#"{"name":"claude-code","version":"2.1.179"}"#);

        server.notify_selection("C:/a.rs".to_string(), "x".to_string(), 22, 22, 0, 77, false);

        let to_new = next_selection(&mut new_cli, &mut new_seen);
        let to_old = next_selection(&mut old_cli, &mut old_seen);
        assert_eq!(to_new["params"]["selection"]["start"]["line"], 21, "2.1.181+ adds 1 itself");
        assert_eq!(to_old["params"]["selection"]["start"]["line"], 22, "older CLIs show it as given");
        assert_eq!(to_new["params"]["selection"]["end"]["character"], 77);
        assert_eq!(to_old["params"]["selection"]["end"]["character"], 77);
    }

    #[test]
    fn a_range_with_no_free_port_reports_zero_instead_of_panicking() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let server = Server::new(port, port);
        assert_eq!(server.start(), 0);
        assert!(!server.is_running());
    }
}
