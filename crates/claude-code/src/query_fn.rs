//! Convenience function for one-off Claude Code queries.
//!
//! This module provides the [`query()`] function, which is the simplest way to
//! send a prompt to Claude Code and receive all response messages. Each call
//! creates a new session, sends the prompt, collects all messages, and closes
//! the connection automatically.
//!
//! For multi-turn conversations, use [`ClaudeSdkClient`](crate::ClaudeSdkClient) instead.

use crate::client::InputPrompt;
use crate::errors::Result;
use crate::internal_client::InternalClient;
use crate::transport::Transport;
use crate::types::{ClaudeAgentOptions, Message};
use futures::Stream;
use futures::stream::LocalBoxStream;
use serde_json::Value;

/// Sends a one-off query to Claude Code and returns all response messages.
///
/// This is the simplest entry point for interacting with Claude Code. It handles
/// the full lifecycle: connecting, initializing, sending the prompt, collecting
/// all messages, and disconnecting.
///
/// # Arguments
///
/// * `prompt` — The input prompt (text or structured messages).
/// * `options` — Optional [`ClaudeAgentOptions`] for configuration. Defaults to
///   [`ClaudeAgentOptions::default()`] if `None`.
/// * `transport` — Optional custom [`Transport`] implementation. If `None`,
///   the default [`SubprocessCliTransport`](crate::SubprocessCliTransport) is used.
///
/// # Returns
///
/// A `Vec<Message>` containing all messages from the interaction, including
/// user echoes, assistant responses, system messages, and the final result.
///
/// # Example
///
/// ```rust,no_run
/// # use claude_code::{query, ClaudeAgentOptions, InputPrompt, Message, PermissionMode};
/// # async fn example() -> claude_code::Result<()> {
/// let messages = query(
///     InputPrompt::Text("Explain Rust ownership".to_string()),
///     Some(ClaudeAgentOptions {
///         permission_mode: Some(PermissionMode::BypassPermissions),
///         max_turns: Some(1),
///         ..Default::default()
///     }),
///     None,
/// ).await?;
///
/// for msg in &messages {
///     if let Message::Result(result) = msg {
///         println!("Cost: ${:.4}", result.total_cost_usd.unwrap_or(0.0));
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub async fn query(
    prompt: InputPrompt,
    options: Option<ClaudeAgentOptions>,
    transport: Option<Box<dyn Transport>>,
) -> Result<Vec<Message>> {
    let options = options.unwrap_or_default();
    let client = InternalClient::new();
    client.process_query(prompt, options, transport).await
}

/// Sends a one-off query using streamed JSON input messages.
///
/// This is a Rust-idiomatic equivalent of Python's `AsyncIterable` prompt mode.
pub async fn query_from_stream<S>(
    prompt: S,
    options: Option<ClaudeAgentOptions>,
    transport: Option<Box<dyn Transport>>,
) -> Result<Vec<Message>>
where
    S: Stream<Item = Value> + Unpin,
{
    let options = options.unwrap_or_default();
    let client = InternalClient::new();
    client
        .process_query_from_stream(prompt, options, transport)
        .await
}

/// Sends a one-off query and returns responses as a stream.
///
/// The returned stream yields parsed [`Message`] values as they arrive.
///
/// Note: the return type is [`LocalBoxStream`], so the stream is not `Send`.
/// Consume it on the same task where it is created.
pub async fn query_stream(
    prompt: InputPrompt,
    options: Option<ClaudeAgentOptions>,
    transport: Option<Box<dyn Transport>>,
) -> Result<LocalBoxStream<'static, Result<Message>>> {
    let options = options.unwrap_or_default();
    let client = InternalClient::new();
    client
        .process_query_as_stream(prompt, options, transport)
        .await
}

/// Sends a one-off query with streamed input and streamed output.
///
/// Note: the return type is [`LocalBoxStream`], so the stream is not `Send`.
/// Consume it on the same task where it is created.
pub async fn query_stream_from_stream<S>(
    prompt: S,
    options: Option<ClaudeAgentOptions>,
    transport: Option<Box<dyn Transport>>,
) -> Result<LocalBoxStream<'static, Result<Message>>>
where
    S: Stream<Item = Value> + Unpin,
{
    let options = options.unwrap_or_default();
    let client = InternalClient::new();
    client
        .process_query_from_stream_as_stream(prompt, options, transport)
        .await
}
