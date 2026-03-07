use std::collections::HashMap;
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
    pub fn with_path(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.path.insert(key.into(), value.into());
        self
    }

    pub fn with_query(mut self, key: impl Into<String>, value: Value) -> Self {
        self.query.insert(key.into(), value);
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

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
    pub event: Option<String>,
    pub id: Option<String>,
    pub retry: Option<u64>,
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
#[derive(Debug, Clone)]
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

#[derive(Debug)]
struct ClientInner {
    http: reqwest::Client,
    base_url: String,
    default_headers: HeaderMap,
}

/// OpenCode API client aligned with official JS SDK request semantics.
#[derive(Debug, Clone)]
pub struct OpencodeClient {
    inner: Arc<ClientInner>,
}

/// Create OpenCode HTTP client.
pub fn create_opencode_client(config: Option<OpencodeClientConfig>) -> Result<OpencodeClient> {
    let config = config.unwrap_or_default();

    let mut default_headers = HeaderMap::new();
    for (k, v) in &config.headers {
        let name = HeaderName::from_bytes(k.as_bytes())?;
        let value = HeaderValue::from_str(v)?;
        default_headers.insert(name, value);
    }

    if let Some(directory) = &config.directory {
        let encoded = encode_component(directory);
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
    pub fn session(&self) -> SessionApi {
        SessionApi {
            client: self.clone(),
        }
    }

    pub fn global(&self) -> GlobalApi {
        GlobalApi {
            client: self.clone(),
        }
    }

    pub fn project(&self) -> ProjectApi {
        ProjectApi {
            client: self.clone(),
        }
    }

    pub fn event(&self) -> EventApi {
        EventApi {
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
            let mut buffer = String::new();
            let mut current = SseEvent::default();

            futures::pin_mut!(byte_stream);
            while let Some(chunk) = byte_stream.next().await {
                let chunk = chunk?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(newline_idx) = buffer.find('\n') {
                    let mut line = buffer[..newline_idx].to_string();
                    buffer.drain(..=newline_idx);

                    if line.ends_with('\r') {
                        line.pop();
                    }

                    if line.is_empty() {
                        if !current.is_empty() {
                            let emitted = std::mem::take(&mut current);
                            yield emitted;
                        }
                        continue;
                    }

                    if line.starts_with(':') {
                        continue;
                    }

                    let (field, value) = match line.split_once(':') {
                        Some((f, v)) => (f, v.trim_start()),
                        None => (line.as_str(), ""),
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
            let name = HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| Error::Other(format!("Invalid header name {k}: {e}")))?;
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

            let value = resolve_path_value(path_params, &key)
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

/// Session endpoint namespace.
#[derive(Debug, Clone)]
pub struct SessionApi {
    client: OpencodeClient,
}

impl SessionApi {
    pub async fn list(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session", options)
            .await
    }

    pub async fn create(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session", options)
            .await
    }

    pub async fn status(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/status", options)
            .await
    }

    pub async fn delete(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::DELETE, "/session/{id}", options)
            .await
    }

    pub async fn get(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{id}", options)
            .await
    }

    pub async fn update(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::PATCH, "/session/{id}", options)
            .await
    }

    pub async fn children(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{id}/children", options)
            .await
    }

    pub async fn todo(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{id}/todo", options)
            .await
    }

    pub async fn init(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/init", options)
            .await
    }

    pub async fn fork(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/fork", options)
            .await
    }

    pub async fn abort(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/abort", options)
            .await
    }

    pub async fn share(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/share", options)
            .await
    }

    pub async fn unshare(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::DELETE, "/session/{id}/share", options)
            .await
    }

    pub async fn diff(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{id}/diff", options)
            .await
    }

    pub async fn summarize(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/summarize", options)
            .await
    }

    pub async fn messages(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{id}/message", options)
            .await
    }

    pub async fn prompt(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/message", options)
            .await
    }

    pub async fn message(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::GET, "/session/{id}/message/{messageID}", options)
            .await
    }

    pub async fn prompt_async(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/prompt_async", options)
            .await
    }

    pub async fn command(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/command", options)
            .await
    }

    pub async fn shell(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/shell", options)
            .await
    }

    pub async fn revert(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/revert", options)
            .await
    }

    pub async fn unrevert(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::POST, "/session/{id}/unrevert", options)
            .await
    }

    pub async fn delete_message(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(Method::DELETE, "/session/{id}/message/{messageID}", options)
            .await
    }

    pub async fn update_part(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(
                Method::PATCH,
                "/session/{id}/message/{messageID}/part/{partID}",
                options,
            )
            .await
    }

    pub async fn delete_part(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(
                Method::DELETE,
                "/session/{id}/message/{messageID}/part/{partID}",
                options,
            )
            .await
    }

    pub async fn respond_permission(&self, options: RequestOptions) -> Result<ApiResponse> {
        self.client
            .request_json(
                Method::POST,
                "/session/{id}/permissions/{permissionID}",
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

fn resolve_path_value<'a>(params: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    if let Some(v) = params.get(key) {
        return Some(v);
    }

    if let Some(v) = params.get(&key.to_ascii_lowercase()) {
        return Some(v);
    }

    // Compatibility with official JS SDK v1 generated shape that often uses {id}.
    if let Some(v) = params.get("id") {
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
