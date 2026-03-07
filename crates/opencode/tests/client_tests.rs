use std::time::Duration;

use futures::StreamExt;
use opencode::{Error, OpencodeClientConfig, RequestOptions, create_opencode_client};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

async fn spawn_single_response_server(response: String) -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");

        let mut read_buf = Vec::new();
        let mut temp = [0u8; 1024];

        loop {
            let n = socket.read(&mut temp).await.expect("read request");
            if n == 0 {
                break;
            }
            read_buf.extend_from_slice(&temp[..n]);

            if read_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let headers_end = read_buf
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .expect("headers end")
                    + 4;

                let head = String::from_utf8_lossy(&read_buf[..headers_end]);
                let content_length = head
                    .lines()
                    .find_map(|line| {
                        let lower = line.to_ascii_lowercase();
                        lower
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().expect("content-length parse"))
                    })
                    .unwrap_or(0);

                let body_len = read_buf.len().saturating_sub(headers_end);
                if body_len >= content_length {
                    break;
                }
            }
        }

        let request_text = String::from_utf8_lossy(&read_buf).to_string();
        let _ = tx.send(request_text);

        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        socket.shutdown().await.expect("shutdown");
    });

    (format!("http://{}", addr), rx)
}

async fn spawn_chunked_response_server(
    chunks: Vec<Vec<u8>>,
) -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");

        let mut read_buf = Vec::new();
        let mut temp = [0u8; 1024];

        loop {
            let n = socket.read(&mut temp).await.expect("read request");
            if n == 0 {
                break;
            }
            read_buf.extend_from_slice(&temp[..n]);

            if read_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        let request_text = String::from_utf8_lossy(&read_buf).to_string();
        let _ = tx.send(request_text);

        for chunk in chunks {
            socket
                .write_all(&chunk)
                .await
                .expect("write response chunk");
        }
        socket.shutdown().await.expect("shutdown");
    });

    (format!("http://{}", addr), rx)
}

#[tokio::test]
async fn session_prompt_posts_expected_path_and_body() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"id\":\"ok\"}";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .session()
        .prompt(
            RequestOptions::default()
                .with_path("id", "ses_123")
                .with_body(json!({
                    "parts": [
                        { "type": "text", "text": "hello" }
                    ]
                })),
        )
        .await
        .expect("prompt");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["id"], "ok");

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("POST /session/ses_123/message HTTP/1.1")
            || request.contains("POST http://") && request.contains("/session/ses_123/message"),
        "unexpected request line: {request}"
    );
    assert!(request.contains("\"type\":\"text\""));
}

#[tokio::test]
async fn directory_header_is_percent_encoded() {
    let response =
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n[]";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        directory: Some("/tmp/中文 路径".to_string()),
        ..Default::default()
    }))
    .expect("client");

    let _ = client
        .session()
        .list(RequestOptions::default())
        .await
        .expect("list");

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("GET /session HTTP/1.1")
            || request.contains("GET http://") && request.contains("/session "),
        "unexpected request line: {request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-opencode-directory: %2ftmp%2f%e4%b8%ad%e6%96%87%20%e8%b7%af%e5%be%84")
    );
}

#[tokio::test]
async fn ascii_directory_header_is_not_percent_encoded() {
    let response =
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n[]";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        directory: Some("/tmp/project".to_string()),
        ..Default::default()
    }))
    .expect("client");

    let _ = client
        .session()
        .list(RequestOptions::default())
        .await
        .expect("list");

    let request = request_rx.await.expect("request capture");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-opencode-directory: /tmp/project")
    );
}

#[tokio::test]
async fn parses_sse_events_from_global_event() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nevent: ping\ndata: {\"ok\":true}\n\ndata: second\n\n";
    let (base_url, _request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        timeout: Duration::from_secs(5),
        ..Default::default()
    }))
    .expect("client");

    let mut stream = client
        .global()
        .event(RequestOptions::default())
        .await
        .expect("global event stream");

    let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("first timeout")
        .expect("first event")
        .expect("first ok");
    assert_eq!(first.event.as_deref(), Some("ping"));
    assert_eq!(first.data, "{\"ok\":true}");

    let second = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("second timeout")
        .expect("second event")
        .expect("second ok");
    assert_eq!(second.event, None);
    assert_eq!(second.data, "second");
}

#[tokio::test]
async fn parses_sse_without_trailing_blank_line() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nevent: ping\ndata: tail-without-terminator";
    let (base_url, _request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        timeout: Duration::from_secs(5),
        ..Default::default()
    }))
    .expect("client");

    let mut stream = client
        .global()
        .event(RequestOptions::default())
        .await
        .expect("global event stream");

    let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("first timeout")
        .expect("first event")
        .expect("first ok");
    assert_eq!(first.event.as_deref(), Some("ping"));
    assert_eq!(first.data, "tail-without-terminator");
}

#[tokio::test]
async fn parses_sse_utf8_when_multibyte_data_is_split_across_chunks() {
    let chunks = vec![
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: "
            .to_vec(),
        vec![0xE4, 0xB8],
        vec![0xAD, 0xE6, 0x96],
        vec![0x87, b'\n', b'\n'],
    ];
    let (base_url, _request_rx) = spawn_chunked_response_server(chunks).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        timeout: Duration::from_secs(5),
        ..Default::default()
    }))
    .expect("client");

    let mut stream = client
        .global()
        .event(RequestOptions::default())
        .await
        .expect("global event stream");

    let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("first timeout")
        .expect("first event")
        .expect("first ok");
    assert_eq!(first.data, "中文");
}

#[tokio::test]
async fn lsp_status_requests_expected_path() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"status\":\"ok\"}";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .lsp()
        .status(RequestOptions::default())
        .await
        .expect("lsp status");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["status"], "ok");

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("GET /lsp HTTP/1.1")
            || request.contains("GET http://") && request.contains("/lsp "),
        "unexpected request line: {request}"
    );
}

#[tokio::test]
async fn vcs_get_requests_expected_path() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"branch\":\"main\"}";

    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .vcs()
        .get(RequestOptions::default())
        .await
        .expect("vcs get");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["branch"], "main");

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("GET /vcs HTTP/1.1")
            || request.contains("GET http://") && request.contains("/vcs "),
        "unexpected request line: {request}"
    );
}

#[tokio::test]
async fn tui_submit_prompt_posts_expected_path_and_body() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}";

    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .tui()
        .submit_prompt(RequestOptions::default().with_body(json!({ "prompt": "hello" })))
        .await
        .expect("tui submit prompt");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["ok"], true);

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("POST /tui/submit-prompt HTTP/1.1")
            || request.contains("POST http://") && request.contains("/tui/submit-prompt"),
        "unexpected request line: {request}"
    );
    assert!(request.contains("\"prompt\":\"hello\""));
}

#[tokio::test]
async fn path_get_requests_expected_path() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"cwd\":\"/tmp\"}";

    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .path()
        .get(RequestOptions::default())
        .await
        .expect("path get");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["cwd"], "/tmp");

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("GET /path HTTP/1.1")
            || request.contains("GET http://") && request.contains("/path "),
        "unexpected request line: {request}"
    );
}

#[tokio::test]
async fn control_next_requests_expected_path() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}";

    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .control()
        .next(RequestOptions::default())
        .await
        .expect("control next");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["ok"], true);

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("GET /tui/control/next HTTP/1.1")
            || request.contains("GET http://") && request.contains("/tui/control/next "),
        "unexpected request line: {request}"
    );
}

#[tokio::test]
async fn command_list_requests_expected_path() {
    let response =
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n[]";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .command()
        .list(RequestOptions::default())
        .await
        .expect("command list");

    assert_eq!(resp.status, 200);
    assert!(resp.data.is_array());

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("GET /command HTTP/1.1")
            || request.contains("GET http://") && request.contains("/command "),
        "unexpected request line: {request}"
    );
}

#[tokio::test]
async fn control_response_posts_expected_path() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .control()
        .response(RequestOptions::default().with_body(json!({"value":"approve"})))
        .await
        .expect("control response");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["ok"], true);

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("POST /tui/control/response HTTP/1.1")
            || request.contains("POST http://") && request.contains("/tui/control/response"),
        "unexpected request line: {request}"
    );
    assert!(request.contains("\"value\":\"approve\""));
}

#[tokio::test]
async fn missing_multi_param_path_field_returns_explicit_error() {
    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url: "http://127.0.0.1:1".to_string(),
        ..Default::default()
    }))
    .expect("client");

    let err = client
        .session()
        .message(RequestOptions::default().with_path("sessionID", "ses_123"))
        .await
        .expect_err("must fail with missing messageID");

    assert!(matches!(err, Error::MissingPathParameter(ref key) if key == "messageID"));
}

#[tokio::test]
async fn session_message_uses_session_and_message_ids() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .session()
        .message(
            RequestOptions::default()
                .with_path("sessionID", "ses_123")
                .with_path("messageID", "msg_456"),
        )
        .await
        .expect("session message");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["ok"], true);

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("GET /session/ses_123/message/msg_456 HTTP/1.1")
            || request.contains("GET http://")
                && request.contains("/session/ses_123/message/msg_456"),
        "unexpected request line: {request}"
    );
}

#[tokio::test]
async fn provider_oauth_authorize_posts_expected_path() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .provider()
        .oauth()
        .authorize(
            RequestOptions::default()
                .with_path("id", "openai")
                .with_body(json!({ "code": "abc123" })),
        )
        .await
        .expect("oauth authorize");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["ok"], true);

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("POST /provider/openai/oauth/authorize HTTP/1.1")
            || request.contains("POST http://")
                && request.contains("/provider/openai/oauth/authorize"),
        "unexpected request line: {request}"
    );
    assert!(request.contains("\"code\":\"abc123\""));
}

#[tokio::test]
async fn auth_set_puts_expected_path_and_body() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .auth()
        .set(
            RequestOptions::default()
                .with_path("id", "openai")
                .with_body(json!({ "api_key": "sk-test" })),
        )
        .await
        .expect("auth set");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["ok"], true);

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("PUT /auth/openai HTTP/1.1")
            || request.contains("PUT http://") && request.contains("/auth/openai"),
        "unexpected request line: {request}"
    );
    assert!(request.contains("\"api_key\":\"sk-test\""));
}

#[tokio::test]
async fn app_log_posts_expected_path_and_body() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .app()
        .log(RequestOptions::default().with_body(json!({
            "level": "info",
            "message": "hello"
        })))
        .await
        .expect("app log");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["ok"], true);

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("POST /log HTTP/1.1")
            || request.contains("POST http://") && request.contains("/log "),
        "unexpected request line: {request}"
    );
    assert!(request.contains("\"message\":\"hello\""));
}

#[tokio::test]
async fn instance_dispose_posts_expected_path() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"disposed\":true}";
    let (base_url, request_rx) = spawn_single_response_server(response.to_string()).await;

    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url,
        ..Default::default()
    }))
    .expect("client");

    let resp = client
        .instance()
        .dispose(RequestOptions::default())
        .await
        .expect("instance dispose");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.data["disposed"], true);

    let request = request_rx.await.expect("request capture");
    assert!(
        request.contains("POST /instance/dispose HTTP/1.1")
            || request.contains("POST http://") && request.contains("/instance/dispose"),
        "unexpected request line: {request}"
    );
}
