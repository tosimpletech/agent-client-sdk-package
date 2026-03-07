//! OpenCode HTTP client and endpoint namespaces.
//!
//! This module provides:
//! - low-level request primitives (`request_json`, `request_sse`)
//! - namespace wrappers for OpenCode endpoints
//! - operation-id dispatch helpers aligned with OpenCode OpenAPI specs

use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_stream::try_stream;
use futures::{Stream, StreamExt};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use serde_json::Value;

use crate::errors::{ApiError, Error, OpencodeSDKError, Result};

// Approximation of JS encodeURIComponent behavior for header/path usage.
const COMPONENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// HTTP request options compatible with OpenCode API endpoints.
#[derive(Debug, Clone, Default)]
pub struct RequestOptions {
    /// Path parameters used to replace placeholders in endpoint template.
    pub path: HashMap<String, String>,
    /// Query string parameters.
    pub query: HashMap<String, Value>,
    /// Request headers.
    pub headers: HashMap<String, String>,
    /// JSON body payload.
    pub body: Option<Value>,
}

impl RequestOptions {
    /// Adds or overrides one path parameter.
    ///
    /// These values are used to render placeholders such as `{sessionID}`.
    pub fn with_path(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.path.insert(key.into(), value.into());
        self
    }

    /// Adds or overrides one query parameter.
    pub fn with_query(mut self, key: impl Into<String>, value: Value) -> Self {
        self.query.insert(key.into(), value);
        self
    }

    /// Adds or overrides one request header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Sets a JSON request body.
    pub fn with_body(mut self, value: Value) -> Self {
        self.body = Some(value);
        self
    }
}

/// Unified JSON response envelope from OpenCode API calls.
#[derive(Debug, Clone)]
pub struct ApiResponse {
    /// Parsed JSON payload. For 204 responses this is `{}`.
    pub data: Value,
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
}

/// Parsed SSE event from OpenCode streaming endpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    /// Event type (`event:` line).
    pub event: Option<String>,
    /// Event id (`id:` line).
    pub id: Option<String>,
    /// Retry hint (`retry:` line).
    pub retry: Option<u64>,
    /// Event data payload (joined `data:` lines).
    pub data: String,
}

impl SseEvent {
    fn is_empty(&self) -> bool {
        self.event.is_none() && self.id.is_none() && self.retry.is_none() && self.data.is_empty()
    }
}

/// Type alias for async SSE stream.
pub type SseStream = Pin<Box<dyn Stream<Item = Result<SseEvent>> + Send>>;

/// Config for creating OpenCode HTTP client.
#[derive(Clone)]
pub struct OpencodeClientConfig {
    /// Base URL for API requests. Defaults to `http://127.0.0.1:4096`.
    pub base_url: String,
    /// Optional project directory mapped to `x-opencode-directory` header.
    pub directory: Option<String>,
    /// Optional default headers.
    pub headers: HashMap<String, String>,
    /// Optional bearer token added as `Authorization: Bearer ...`.
    pub bearer_token: Option<String>,
    /// Request timeout.
    pub timeout: Duration,
}

impl fmt::Debug for OpencodeClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpencodeClientConfig")
            .field("base_url", &self.base_url)
            .field("directory", &self.directory)
            .field("headers", &"<redacted>")
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl Default for OpencodeClientConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:4096".to_string(),
            directory: None,
            headers: HashMap::new(),
            bearer_token: None,
            timeout: Duration::from_secs(60),
        }
    }
}

struct ClientInner {
    http: reqwest::Client,
    base_url: String,
    default_headers: HeaderMap,
}

impl fmt::Debug for ClientInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientInner")
            .field("base_url", &self.base_url)
            .field("default_headers", &"<redacted>")
            .finish()
    }
}

/// OpenCode API client aligned with official JS SDK request semantics.
#[derive(Clone)]
pub struct OpencodeClient {
    inner: Arc<ClientInner>,
}

impl fmt::Debug for OpencodeClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpencodeClient")
            .field("base_url", &self.inner.base_url)
            .finish()
    }
}

/// Creates an OpenCode HTTP client from config.
///
/// The client applies:
/// - default headers from `config.headers`
/// - `Authorization: Bearer ...` when `bearer_token` is set
/// - `x-opencode-directory` when `directory` is set
/// - a request timeout from `config.timeout`
pub fn create_opencode_client(config: Option<OpencodeClientConfig>) -> Result<OpencodeClient> {
    let config = config.unwrap_or_default();

    let mut default_headers = HeaderMap::new();
    for (k, v) in &config.headers {
        let name = HeaderName::from_bytes(k.as_bytes())?;
        let value = HeaderValue::from_str(v)?;
        default_headers.insert(name, value);
    }

    if let Some(directory) = &config.directory {
        let encoded = encode_directory_header(directory);
        default_headers.insert(
            HeaderName::from_static("x-opencode-directory"),
            HeaderValue::from_str(&encoded)?,
        );
    }

    if let Some(token) = &config.bearer_token {
        default_headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {token}"))?,
        );
    }

    let http = reqwest::Client::builder()
        .timeout(config.timeout)
        .default_headers(default_headers.clone())
        .build()?;

    Ok(OpencodeClient {
        inner: Arc::new(ClientInner {
            http,
            base_url: config.base_url,
            default_headers,
        }),
    })
}

impl OpencodeClient {
    /// Returns session endpoint APIs (`/session/...`).
    pub fn session(&self) -> SessionApi {
        SessionApi {
            client: self.clone(),
        }
    }

    /// Returns app-level endpoint APIs.
    pub fn app(&self) -> AppApi {
        AppApi {
            client: self.clone(),
        }
    }

    /// Returns global endpoint APIs.
    pub fn global(&self) -> GlobalApi {
        GlobalApi {
            client: self.clone(),
        }
    }

    /// Returns command endpoint APIs.
    pub fn command(&self) -> CommandApi {
        CommandApi {
            client: self.clone(),
        }
    }

    /// Returns config endpoint APIs.
    pub fn config(&self) -> ConfigApi {
        ConfigApi {
            client: self.clone(),
        }
    }

    /// Returns project endpoint APIs.
    pub fn project(&self) -> ProjectApi {
        ProjectApi {
            client: self.clone(),
        }
    }

    /// Returns path endpoint APIs.
    pub fn path(&self) -> PathApi {
        PathApi {
            client: self.clone(),
        }
    }

    /// Returns file endpoint APIs.
    pub fn file(&self) -> FileApi {
        FileApi {
            client: self.clone(),
        }
    }

    /// Returns LSP endpoint APIs.
    pub fn lsp(&self) -> LspApi {
        LspApi {
            client: self.clone(),
        }
    }

    /// Returns tool endpoint APIs.
    pub fn tool(&self) -> ToolApi {
        ToolApi {
            client: self.clone(),
        }
    }

    /// Returns provider endpoint APIs.
    pub fn provider(&self) -> ProviderApi {
        ProviderApi {
            client: self.clone(),
        }
    }

    /// Returns auth endpoint APIs.
    pub fn auth(&self) -> AuthApi {
        AuthApi {
            client: self.clone(),
        }
    }

    /// Returns MCP endpoint APIs.
    pub fn mcp(&self) -> McpApi {
        McpApi {
            client: self.clone(),
        }
    }

    /// Returns PTY endpoint APIs.
    pub fn pty(&self) -> PtyApi {
        PtyApi {
            client: self.clone(),
        }
    }

    /// Returns event endpoint APIs.
    pub fn event(&self) -> EventApi {
        EventApi {
            client: self.clone(),
        }
    }

    /// Returns formatter endpoint APIs.
    pub fn formatter(&self) -> FormatterApi {
        FormatterApi {
            client: self.clone(),
        }
    }

    /// Returns find endpoint APIs.
    pub fn find(&self) -> FindApi {
        FindApi {
            client: self.clone(),
        }
    }

    /// Returns instance endpoint APIs.
    pub fn instance(&self) -> InstanceApi {
        InstanceApi {
            client: self.clone(),
        }
    }

    /// Returns VCS endpoint APIs.
    pub fn vcs(&self) -> VcsApi {
        VcsApi {
            client: self.clone(),
        }
    }

    /// Returns TUI endpoint APIs.
    pub fn tui(&self) -> TuiApi {
        TuiApi {
            client: self.clone(),
        }
    }

    /// Backward-compatible shorthand for TUI control endpoints.
    pub fn control(&self) -> ControlApi {
        ControlApi {
            client: self.clone(),
        }
    }

    /// Execute any OpenCode operation by official `operationId`.
    pub async fn call_operation(
        &self,
        operation_id: &str,
        options: RequestOptions,
    ) -> Result<ApiResponse> {
        let (method, path, is_sse) = operation_spec(operation_id).ok_or_else(|| {
            Error::OpencodeSDK(OpencodeSDKError::new(format!(
                "Unknown operation id: {operation_id}"
            )))
        })?;

        if is_sse {
            return Err(Error::OpencodeSDK(OpencodeSDKError::new(format!(
                "Operation {operation_id} is SSE; use call_operation_sse"
            ))));
        }

        self.request_json(method, path, options).await
    }

    /// Execute SSE operation by official `operationId` (`global.event` / `event.subscribe`).
    pub async fn call_operation_sse(
        &self,
        operation_id: &str,
        options: RequestOptions,
    ) -> Result<SseStream> {
        let (method, path, is_sse) = operation_spec(operation_id).ok_or_else(|| {
            Error::OpencodeSDK(OpencodeSDKError::new(format!(
                "Unknown operation id: {operation_id}"
            )))
        })?;

        if !is_sse {
            return Err(Error::OpencodeSDK(OpencodeSDKError::new(format!(
                "Operation {operation_id} is not SSE; use call_operation"
            ))));
        }

        self.request_sse(method, path, options).await
    }

    /// Sends one HTTP request and returns a parsed JSON (or text) response envelope.
    ///
    /// For `2xx` responses:
    /// - `204` or empty body -> `{}` payload
    /// - valid JSON body -> parsed JSON
    /// - non-JSON body -> string payload
    ///
    /// For non-`2xx` responses, returns [`Error::Api`] with status and raw body.
    pub async fn request_json(
        &self,
        method: Method,
        path_template: &str,
        options: RequestOptions,
    ) -> Result<ApiResponse> {
        let response = self.send_request(method, path_template, options).await?;
        let status = response.status().as_u16();
        let headers = headers_to_map(response.headers());
        let bytes = response.bytes().await?;

        if (200..300).contains(&status) {
            let data = if status == 204 || bytes.is_empty() {
                serde_json::json!({})
            } else {
                parse_success_body(&bytes)
            };

            return Ok(ApiResponse {
                data,
                status,
                headers,
            });
        }

        let body_text = String::from_utf8_lossy(&bytes).to_string();
        Err(Error::Api(ApiError {
            status,
            body: body_text,
        }))
    }

    /// Sends one HTTP request and parses the response as Server-Sent Events.
    ///
    /// The parser supports:
    /// - split UTF-8 across chunks
    /// - multi-line `data:` fields
    /// - trailing final lines without a terminating blank line
    pub async fn request_sse(
        &self,
        method: Method,
        path_template: &str,
        options: RequestOptions,
    ) -> Result<SseStream> {
        let response = self.send_request(method, path_template, options).await?;
        let status = response.status().as_u16();

        if !(200..300).contains(&status) {
            let body_text = response.text().await.unwrap_or_default();
            return Err(Error::Api(ApiError {
                status,
                body: body_text,
            }));
        }

        let byte_stream = response.bytes_stream();
        let out = try_stream! {
            let mut buffer = Vec::<u8>::new();
            let mut current = SseEvent::default();

            futures::pin_mut!(byte_stream);
            while let Some(chunk) = byte_stream.next().await {
                let chunk = chunk?;
                buffer.extend_from_slice(&chunk);

                while let Some(newline_idx) = buffer.iter().position(|b| *b == b'\n') {
                    let mut line = buffer.drain(..=newline_idx).collect::<Vec<_>>();
                    if matches!(line.last(), Some(b'\n')) {
                        line.pop();
                    }
                    if matches!(line.last(), Some(b'\r')) {
                        line.pop();
                    }

                    let line = String::from_utf8_lossy(&line).into_owned();

                    if line.is_empty() {
                        if !current.is_empty() {
                            let emitted = std::mem::take(&mut current);
                            yield emitted;
                        }
                        continue;
                    }

                    apply_sse_line(&line, &mut current);
                }
            }

            if !buffer.is_empty() {
                if matches!(buffer.last(), Some(b'\r')) {
                    buffer.pop();
                }
                let line = String::from_utf8_lossy(&buffer).into_owned();
                if !line.is_empty() {
                    apply_sse_line(&line, &mut current);
                }
            }

            if !current.is_empty() {
                yield current;
            }
        };

        Ok(Box::pin(out))
    }

    async fn send_request(
        &self,
        method: Method,
        path_template: &str,
        options: RequestOptions,
    ) -> Result<reqwest::Response> {
        let url = self.build_url(path_template, &options.path, &options.query)?;

        let mut req = self.inner.http.request(method, url);

        let mut merged_headers = self.inner.default_headers.clone();
        for (k, v) in &options.headers {
            let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| {
                Error::OpencodeSDK(OpencodeSDKError::new(format!(
                    "Invalid header name {k}: {e}"
                )))
            })?;
            let value = HeaderValue::from_str(v)?;
            merged_headers.insert(name, value);
        }
        req = req.headers(merged_headers);

        if let Some(body) = options.body {
            req = req.json(&body);
        }

        Ok(req.send().await?)
    }

    fn build_url(
        &self,
        path_template: &str,
        path_params: &HashMap<String, String>,
        query_params: &HashMap<String, Value>,
    ) -> Result<Url> {
        let allow_id_fallback = path_template.matches('{').count() == 1;
        let mut rendered_path = String::new();
        let mut chars = path_template.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch != '{' {
                rendered_path.push(ch);
                continue;
            }

            let mut key = String::new();
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
                key.push(next);
            }

            if key.is_empty() {
                return Err(Error::OpencodeSDK(OpencodeSDKError::new(
                    "Empty path parameter name in template",
                )));
            }

            let value = resolve_path_value(path_params, &key, allow_id_fallback)
                .ok_or_else(|| Error::MissingPathParameter(key.clone()))?;
            rendered_path.push_str(&encode_component(value));
        }

        let base = self.inner.base_url.trim_end_matches('/');
        let suffix = rendered_path.trim_start_matches('/');
        let full = format!("{base}/{suffix}");

        let mut url = Url::parse(&full).map_err(|e| {
            Error::OpencodeSDK(OpencodeSDKError::new(format!("Invalid URL {full}: {e}")))
        })?;

        let mut pairs = Vec::new();
        for (key, value) in query_params {
            append_query_value(&mut pairs, key, value);
        }
        if !pairs.is_empty() {
            let mut qp = url.query_pairs_mut();
            for (key, value) in pairs {
                qp.append_pair(&key, &value);
            }
        }

        Ok(url)
    }
}

/// Find endpoint namespace.
#[derive(Debug, Clone)]
pub struct FindApi {
    client: OpencodeClient,
}

impl FindApi {
    /// Searches text content.
    pub async fn text(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/find", options)
            .await
    }

    /// Searches files by query/pattern.
    pub async fn files(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/find/file", options)
            .await
    }

    /// Searches symbols.
    pub async fn symbols(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/find/symbol", options)
            .await
    }
}

/// Session endpoint namespace.
#[derive(Debug, Clone)]
pub struct SessionApi {
    client: OpencodeClient,
}

impl SessionApi {
    /// Lists sessions.
    pub async fn list(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session", options)
            .await
    }

    /// Creates a new session.
    pub async fn create(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session", options)
            .await
    }

    /// Returns session runtime status.
    pub async fn status(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/status", options)
            .await
    }

    /// Deletes a session.
    pub async fn delete(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::DELETE, "/session/{sessionID}", options)
            .await
    }

    /// Gets one session by id.
    pub async fn get(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{sessionID}", options)
            .await
    }

    /// Updates mutable session fields.
    pub async fn update(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::PATCH, "/session/{sessionID}", options)
            .await
    }

    /// Lists children for a session.
    pub async fn children(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{sessionID}/children", options)
            .await
    }

    /// Returns session todo items.
    pub async fn todo(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{sessionID}/todo", options)
            .await
    }

    /// Initializes a session.
    pub async fn init(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/init", options)
            .await
    }

    /// Forks a session.
    pub async fn fork(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/fork", options)
            .await
    }

    /// Aborts the active run in a session.
    pub async fn abort(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/abort", options)
            .await
    }

    /// Shares a session.
    pub async fn share(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/share", options)
            .await
    }

    /// Revokes sharing for a session.
    pub async fn unshare(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::DELETE, "/session/{sessionID}/share", options)
            .await
    }

    /// Gets session diff.
    pub async fn diff(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{sessionID}/diff", options)
            .await
    }

    /// Triggers session summarization.
    pub async fn summarize(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/summarize", options)
            .await
    }

    /// Lists messages in a session.
    pub async fn messages(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{sessionID}/message", options)
            .await
    }

    /// Sends a prompt message to a session.
    pub async fn prompt(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/message", options)
            .await
    }

    /// Gets one message by id.
    pub async fn message(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(
                Method::GET,
                "/session/{sessionID}/message/{messageID}",
                options,
            )
            .await
    }

    /// Enqueues an async prompt run.
    pub async fn prompt_async(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/prompt_async", options)
            .await
    }

    /// Sends a command to a session.
    pub async fn command(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/command", options)
            .await
    }

    /// Executes a shell action in a session.
    pub async fn shell(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/shell", options)
            .await
    }

    /// Reverts one message in session history.
    pub async fn revert(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/revert", options)
            .await
    }

    /// Restores all reverted messages.
    pub async fn unrevert(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{sessionID}/unrevert", options)
            .await
    }

    /// Deletes one message.
    pub async fn delete_message(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(
                Method::DELETE,
                "/session/{sessionID}/message/{messageID}",
                options,
            )
            .await
    }

    /// Updates one message part.
    pub async fn update_part(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(
                Method::PATCH,
                "/session/{sessionID}/message/{messageID}/part/{partID}",
                options,
            )
            .await
    }

    /// Deletes one message part.
    pub async fn delete_part(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(
                Method::DELETE,
                "/session/{sessionID}/message/{messageID}/part/{partID}",
                options,
            )
            .await
    }

    /// Responds to a permission request under one session.
    pub async fn respond_permission(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(
                Method::POST,
                "/session/{sessionID}/permissions/{permissionID}",
                options,
            )
            .await
    }
}

/// Global endpoint namespace.
#[derive(Debug, Clone)]
pub struct GlobalApi {
    client: OpencodeClient,
}

impl GlobalApi {
    pub async fn health(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/global/health", options)
            .await
    }

    pub async fn dispose(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/global/dispose", options)
            .await
    }

    pub async fn config_get(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/global/config", options)
            .await
    }

    pub async fn config_update(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::PATCH, "/global/config", options)
            .await
    }

    pub async fn event(&self, options: RequestOptions) -> Result<SseStream> {
        self.client
            .request_sse(Method::GET, "/global/event", options)
            .await
    }
}

/// App endpoint namespace.
#[derive(Debug, Clone)]
pub struct AppApi {
    client: OpencodeClient,
}

impl AppApi {
    pub async fn agents(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("app.agents", options).await
    }

    pub async fn log(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("app.log", options).await
    }

    pub async fn skills(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("app.skills", options).await
    }
}

/// Command endpoint namespace.
#[derive(Debug, Clone)]
pub struct CommandApi {
    client: OpencodeClient,
}

impl CommandApi {
    pub async fn list(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("command.list", options).await
    }
}

/// Instance endpoint namespace.
#[derive(Debug, Clone)]
pub struct InstanceApi {
    client: OpencodeClient,
}

impl InstanceApi {
    pub async fn dispose(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("instance.dispose", options)
            .await
    }
}

/// Config endpoint namespace.
#[derive(Debug, Clone)]
pub struct ConfigApi {
    client: OpencodeClient,
}

impl ConfigApi {
    pub async fn get(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("config.get", options).await
    }

    pub async fn update(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("config.update", options).await
    }

    pub async fn providers(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("config.providers", options)
            .await
    }
}

/// Project endpoint namespace.
#[derive(Debug, Clone)]
pub struct ProjectApi {
    client: OpencodeClient,
}

impl ProjectApi {
    pub async fn list(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/project", options)
            .await
    }

    pub async fn current(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/project/current", options)
            .await
    }

    pub async fn update(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::PATCH, "/project/{projectID}", options)
            .await
    }
}

/// Path endpoint namespace.
#[derive(Debug, Clone)]
pub struct PathApi {
    client: OpencodeClient,
}

impl PathApi {
    pub async fn get(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("path.get", options).await
    }
}

/// File endpoint namespace.
#[derive(Debug, Clone)]
pub struct FileApi {
    client: OpencodeClient,
}

impl FileApi {
    pub async fn list(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/file", options)
            .await
    }

    pub async fn read(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/file/content", options)
            .await
    }
}

/// LSP endpoint namespace.
#[derive(Debug, Clone)]
pub struct LspApi {
    client: OpencodeClient,
}

impl LspApi {
    pub async fn status(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.request_json(Method::GET, "/lsp", options).await
    }
}

/// Tool endpoint namespace.
#[derive(Debug, Clone)]
pub struct ToolApi {
    client: OpencodeClient,
}

impl ToolApi {
    pub async fn ids(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/experimental/tool/ids", options)
            .await
    }

    pub async fn list(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/experimental/tool", options)
            .await
    }
}

/// Auth endpoint namespace.
#[derive(Debug, Clone)]
pub struct AuthApi {
    client: OpencodeClient,
}

impl AuthApi {
    pub async fn set(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("auth.set", options).await
    }

    pub async fn remove(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("auth.remove", options).await
    }
}

/// Provider endpoint namespace.
#[derive(Debug, Clone)]
pub struct ProviderApi {
    client: OpencodeClient,
}

impl ProviderApi {
    pub async fn list(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/provider", options)
            .await
    }

    pub async fn auth(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/provider/auth", options)
            .await
    }

    pub fn oauth(&self) -> OauthApi {
        OauthApi {
            client: self.client.clone(),
        }
    }
}

/// OAuth endpoint namespace under provider routes.
#[derive(Debug, Clone)]
pub struct OauthApi {
    client: OpencodeClient,
}

impl OauthApi {
    pub async fn authorize(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/provider/{id}/oauth/authorize", options)
            .await
    }

    pub async fn callback(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/provider/{id}/oauth/callback", options)
            .await
    }
}

/// MCP endpoint namespace.
#[derive(Debug, Clone)]
pub struct McpApi {
    client: OpencodeClient,
}

impl McpApi {
    pub async fn status(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.request_json(Method::GET, "/mcp", options).await
    }

    pub async fn add(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/mcp", options)
            .await
    }

    pub async fn connect(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/mcp/{name}/connect", options)
            .await
    }

    pub async fn disconnect(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/mcp/{name}/disconnect", options)
            .await
    }

    pub fn auth(&self) -> McpAuthApi {
        McpAuthApi {
            client: self.client.clone(),
        }
    }
}

/// MCP auth endpoint namespace.
#[derive(Debug, Clone)]
pub struct McpAuthApi {
    client: OpencodeClient,
}

impl McpAuthApi {
    pub async fn remove(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::DELETE, "/mcp/{name}/auth", options)
            .await
    }

    pub async fn start(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/mcp/{name}/auth", options)
            .await
    }

    pub async fn callback(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/mcp/{name}/auth/callback", options)
            .await
    }

    pub async fn authenticate(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/mcp/{name}/auth/authenticate", options)
            .await
    }
}

/// PTY endpoint namespace.
#[derive(Debug, Clone)]
pub struct PtyApi {
    client: OpencodeClient,
}

impl PtyApi {
    pub async fn list(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.request_json(Method::GET, "/pty", options).await
    }

    pub async fn create(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/pty", options)
            .await
    }

    pub async fn remove(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::DELETE, "/pty/{ptyID}", options)
            .await
    }

    pub async fn get(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/pty/{ptyID}", options)
            .await
    }

    pub async fn update(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::PUT, "/pty/{ptyID}", options)
            .await
    }

    pub async fn connect(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/pty/{ptyID}/connect", options)
            .await
    }
}

/// Event endpoint namespace.
#[derive(Debug, Clone)]
pub struct EventApi {
    client: OpencodeClient,
}

impl EventApi {
    pub async fn subscribe(&self, options: RequestOptions) -> Result<SseStream> {
        self.client
            .request_sse(Method::GET, "/event", options)
            .await
    }
}

/// Formatter endpoint namespace.
#[derive(Debug, Clone)]
pub struct FormatterApi {
    client: OpencodeClient,
}

impl FormatterApi {
    pub async fn status(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("formatter.status", options)
            .await
    }
}

/// VCS endpoint namespace.
#[derive(Debug, Clone)]
pub struct VcsApi {
    client: OpencodeClient,
}

impl VcsApi {
    pub async fn get(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("vcs.get", options).await
    }
}

/// TUI endpoint namespace.
#[derive(Debug, Clone)]
pub struct TuiApi {
    client: OpencodeClient,
}

impl TuiApi {
    pub async fn append_prompt(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("tui.appendPrompt", options)
            .await
    }

    pub async fn clear_prompt(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("tui.clearPrompt", options).await
    }

    pub async fn execute_command(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("tui.executeCommand", options)
            .await
    }

    pub async fn open_help(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("tui.openHelp", options).await
    }

    pub async fn open_models(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("tui.openModels", options).await
    }

    pub async fn open_sessions(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("tui.openSessions", options)
            .await
    }

    pub async fn open_themes(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("tui.openThemes", options).await
    }

    pub async fn publish(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("tui.publish", options).await
    }

    pub async fn select_session(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("tui.selectSession", options)
            .await
    }

    pub async fn show_toast(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client.call_operation("tui.showToast", options).await
    }

    pub async fn submit_prompt(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("tui.submitPrompt", options)
            .await
    }

    pub fn control(&self) -> TuiControlApi {
        TuiControlApi {
            client: self.client.clone(),
        }
    }
}

/// TUI control endpoint namespace.
#[derive(Debug, Clone)]
pub struct TuiControlApi {
    client: OpencodeClient,
}

impl TuiControlApi {
    pub async fn next(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("tui.control.next", options)
            .await
    }

    pub async fn response(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .call_operation("tui.control.response", options)
            .await
    }
}

/// Backward-compatible alias for top-level control API access.
pub type ControlApi = TuiControlApi;

fn apply_sse_line(line: &str, current: &mut SseEvent) {
    if line.starts_with(':') {
        return;
    }

    let (field, value) = match line.split_once(':') {
        Some((f, v)) => (f, v.trim_start()),
        None => (line, ""),
    };

    match field {
        "event" => current.event = Some(value.to_string()),
        "id" => current.id = Some(value.to_string()),
        "retry" => {
            if let Ok(v) = value.parse::<u64>() {
                current.retry = Some(v);
            }
        }
        "data" => {
            if !current.data.is_empty() {
                current.data.push('\n');
            }
            current.data.push_str(value);
        }
        _ => {}
    }
}

fn parse_success_body(bytes: &[u8]) -> Value {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(json) => json,
        Err(_) => Value::String(String::from_utf8_lossy(bytes).to_string()),
    }
}

fn headers_to_map(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|value| (k.as_str().to_string(), value.to_string()))
        })
        .collect()
}

fn encode_component(value: &str) -> String {
    utf8_percent_encode(value, COMPONENT_ENCODE_SET).to_string()
}

fn encode_directory_header(value: &str) -> String {
    if value.is_ascii() {
        value.to_string()
    } else {
        encode_component(value)
    }
}

fn resolve_path_value<'a>(
    params: &'a HashMap<String, String>,
    key: &str,
    allow_id_fallback: bool,
) -> Option<&'a str> {
    if let Some(v) = params.get(key) {
        return Some(v);
    }

    if let Some(v) = params.get(&key.to_ascii_lowercase()) {
        return Some(v);
    }

    if key.ends_with("ID") {
        let alt = key.trim_end_matches("ID");
        if let Some(v) = params.get(alt) {
            return Some(v);
        }

        let snake = to_snake_case(alt);
        if let Some(v) = params.get(&snake) {
            return Some(v);
        }

        let snake_id = format!("{}_id", snake);
        if let Some(v) = params.get(&snake_id) {
            return Some(v);
        }
    }

    // Compatibility with official JS SDK v1 shape that frequently uses {id}
    // for single-parameter routes. Avoid this fallback on multi-parameter
    // routes (e.g. {sessionID}/{messageID}) to prevent accidental substitution.
    if allow_id_fallback {
        if let Some(v) = params.get("id") {
            return Some(v);
        }
    }

    None
}

fn to_snake_case(input: &str) -> String {
    let mut out = String::new();
    for (idx, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if idx > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn append_query_value(out: &mut Vec<(String, String)>, key: &str, value: &Value) {
    match value {
        Value::Null => {}
        Value::Bool(v) => {
            out.push((
                key.to_string(),
                if *v { "true" } else { "false" }.to_string(),
            ));
        }
        Value::Number(v) => {
            out.push((key.to_string(), v.to_string()));
        }
        Value::String(v) => {
            out.push((key.to_string(), v.clone()));
        }
        Value::Array(items) => {
            for item in items {
                append_query_value(out, key, item);
            }
        }
        Value::Object(_) => {
            out.push((key.to_string(), value.to_string()));
        }
    }
}

fn operation_spec(operation_id: &str) -> Option<(Method, &'static str, bool)> {
    let (method, path, sse) = match operation_id {
        "app.agents" => ("GET", "/agent", false),
        "app.log" => ("POST", "/log", false),
        "app.skills" => ("GET", "/skill", false),
        "auth.remove" => ("DELETE", "/auth/{providerID}", false),
        "auth.set" => ("PUT", "/auth/{providerID}", false),
        "command.list" => ("GET", "/command", false),
        "config.get" => ("GET", "/config", false),
        "config.providers" => ("GET", "/config/providers", false),
        "config.update" => ("PATCH", "/config", false),
        "event.subscribe" => ("GET", "/event", true),
        "experimental.resource.list" => ("GET", "/experimental/resource", false),
        "experimental.session.list" => ("GET", "/experimental/session", false),
        "experimental.workspace.create" => ("POST", "/experimental/workspace", false),
        "experimental.workspace.list" => ("GET", "/experimental/workspace", false),
        "experimental.workspace.remove" => ("DELETE", "/experimental/workspace/{id}", false),
        "file.list" => ("GET", "/file", false),
        "file.read" => ("GET", "/file/content", false),
        "file.status" => ("GET", "/file/status", false),
        "find.files" => ("GET", "/find/file", false),
        "find.symbols" => ("GET", "/find/symbol", false),
        "find.text" => ("GET", "/find", false),
        "formatter.status" => ("GET", "/formatter", false),
        "global.config.get" => ("GET", "/global/config", false),
        "global.config.update" => ("PATCH", "/global/config", false),
        "global.dispose" => ("POST", "/global/dispose", false),
        "global.event" => ("GET", "/global/event", true),
        "global.health" => ("GET", "/global/health", false),
        "instance.dispose" => ("POST", "/instance/dispose", false),
        "lsp.status" => ("GET", "/lsp", false),
        "mcp.add" => ("POST", "/mcp", false),
        "mcp.auth.authenticate" => ("POST", "/mcp/{name}/auth/authenticate", false),
        "mcp.auth.callback" => ("POST", "/mcp/{name}/auth/callback", false),
        "mcp.auth.remove" => ("DELETE", "/mcp/{name}/auth", false),
        "mcp.auth.start" => ("POST", "/mcp/{name}/auth", false),
        "mcp.connect" => ("POST", "/mcp/{name}/connect", false),
        "mcp.disconnect" => ("POST", "/mcp/{name}/disconnect", false),
        "mcp.status" => ("GET", "/mcp", false),
        "part.delete" => (
            "DELETE",
            "/session/{sessionID}/message/{messageID}/part/{partID}",
            false,
        ),
        "part.update" => (
            "PATCH",
            "/session/{sessionID}/message/{messageID}/part/{partID}",
            false,
        ),
        "path.get" => ("GET", "/path", false),
        "permission.list" => ("GET", "/permission", false),
        "permission.reply" => ("POST", "/permission/{requestID}/reply", false),
        "permission.respond" => (
            "POST",
            "/session/{sessionID}/permissions/{permissionID}",
            false,
        ),
        "project.current" => ("GET", "/project/current", false),
        "project.list" => ("GET", "/project", false),
        "project.update" => ("PATCH", "/project/{projectID}", false),
        "provider.auth" => ("GET", "/provider/auth", false),
        "provider.list" => ("GET", "/provider", false),
        "provider.oauth.authorize" => ("POST", "/provider/{providerID}/oauth/authorize", false),
        "provider.oauth.callback" => ("POST", "/provider/{providerID}/oauth/callback", false),
        "pty.connect" => ("GET", "/pty/{ptyID}/connect", false),
        "pty.create" => ("POST", "/pty", false),
        "pty.get" => ("GET", "/pty/{ptyID}", false),
        "pty.list" => ("GET", "/pty", false),
        "pty.remove" => ("DELETE", "/pty/{ptyID}", false),
        "pty.update" => ("PUT", "/pty/{ptyID}", false),
        "question.list" => ("GET", "/question", false),
        "question.reject" => ("POST", "/question/{requestID}/reject", false),
        "question.reply" => ("POST", "/question/{requestID}/reply", false),
        "session.abort" => ("POST", "/session/{sessionID}/abort", false),
        "session.children" => ("GET", "/session/{sessionID}/children", false),
        "session.command" => ("POST", "/session/{sessionID}/command", false),
        "session.create" => ("POST", "/session", false),
        "session.delete" => ("DELETE", "/session/{sessionID}", false),
        "session.deleteMessage" => ("DELETE", "/session/{sessionID}/message/{messageID}", false),
        "session.diff" => ("GET", "/session/{sessionID}/diff", false),
        "session.fork" => ("POST", "/session/{sessionID}/fork", false),
        "session.get" => ("GET", "/session/{sessionID}", false),
        "session.init" => ("POST", "/session/{sessionID}/init", false),
        "session.list" => ("GET", "/session", false),
        "session.message" => ("GET", "/session/{sessionID}/message/{messageID}", false),
        "session.messages" => ("GET", "/session/{sessionID}/message", false),
        "session.prompt" => ("POST", "/session/{sessionID}/message", false),
        "session.prompt_async" => ("POST", "/session/{sessionID}/prompt_async", false),
        "session.revert" => ("POST", "/session/{sessionID}/revert", false),
        "session.share" => ("POST", "/session/{sessionID}/share", false),
        "session.shell" => ("POST", "/session/{sessionID}/shell", false),
        "session.status" => ("GET", "/session/status", false),
        "session.summarize" => ("POST", "/session/{sessionID}/summarize", false),
        "session.todo" => ("GET", "/session/{sessionID}/todo", false),
        "session.unrevert" => ("POST", "/session/{sessionID}/unrevert", false),
        "session.unshare" => ("DELETE", "/session/{sessionID}/share", false),
        "session.update" => ("PATCH", "/session/{sessionID}", false),
        "tool.ids" => ("GET", "/experimental/tool/ids", false),
        "tool.list" => ("GET", "/experimental/tool", false),
        "tui.appendPrompt" => ("POST", "/tui/append-prompt", false),
        "tui.clearPrompt" => ("POST", "/tui/clear-prompt", false),
        "tui.control.next" => ("GET", "/tui/control/next", false),
        "tui.control.response" => ("POST", "/tui/control/response", false),
        "tui.executeCommand" => ("POST", "/tui/execute-command", false),
        "tui.openHelp" => ("POST", "/tui/open-help", false),
        "tui.openModels" => ("POST", "/tui/open-models", false),
        "tui.openSessions" => ("POST", "/tui/open-sessions", false),
        "tui.openThemes" => ("POST", "/tui/open-themes", false),
        "tui.publish" => ("POST", "/tui/publish", false),
        "tui.selectSession" => ("POST", "/tui/select-session", false),
        "tui.showToast" => ("POST", "/tui/show-toast", false),
        "tui.submitPrompt" => ("POST", "/tui/submit-prompt", false),
        "vcs.get" => ("GET", "/vcs", false),
        "worktree.create" => ("POST", "/experimental/worktree", false),
        "worktree.list" => ("GET", "/experimental/worktree", false),
        "worktree.remove" => ("DELETE", "/experimental/worktree", false),
        "worktree.reset" => ("POST", "/experimental/worktree/reset", false),
        _ => return None,
    };

    Some((Method::from_bytes(method.as_bytes()).ok()?, path, sse))
}
