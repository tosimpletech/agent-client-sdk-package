# OpenCode Rust SDK

与官方 OpenCode JavaScript SDK（`@opencode-ai/sdk` `v1.2.20`）对齐的 Rust 版本。

## 功能

- 启停本地 OpenCode 服务：`create_opencode_server`
- 启停 OpenCode TUI：`create_opencode_tui`
- 创建 OpenCode HTTP 客户端：`create_opencode_client`
- 一体化启动：`create_opencode`（返回 server + client）
- 提供 `session/global/project/provider/oauth/event/lsp/pty` 常用接口封装
- 支持按 `operationId` 通用调用：`call_operation` / `call_operation_sse`

## 安装

```toml
[dependencies]
opencode-client-sdk = "1.2.20"
```

## 快速开始

```rust,no_run
use opencode::{create_opencode, RequestOptions};
use serde_json::json;

# async fn run() -> opencode::Result<()> {
let mut app = create_opencode(None).await?;

let session = app
    .client
    .session()
    .create(RequestOptions::default())
    .await?;

let session_id = session.data["id"].as_str().unwrap();

let response = app
    .client
    .session()
    .prompt(
        RequestOptions::default()
            .with_path("id", session_id)
            .with_body(json!({
                "parts": [
                    { "type": "text", "text": "总结这个仓库" }
                ]
            })),
    )
    .await?;

println!("status={} data={}", response.status, response.data);

app.server.close().await?;
# Ok(())
# }
```

## 说明

- 为兼容 OpenCode schema 的快速迭代，客户端 body/response 以 `serde_json::Value` 为主。
- 当设置 `OpencodeClientConfig.directory` 时，会自动附加 `x-opencode-directory` 请求头。
