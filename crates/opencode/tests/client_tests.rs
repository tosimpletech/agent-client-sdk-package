use std::time::Duration;

use futures::StreamExt;
use opencode::{OpencodeClientConfig, RequestOptions, create_opencode_client};
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
