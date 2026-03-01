use crate::client::InputPrompt;
use crate::errors::Result;
use crate::internal_client::InternalClient;
use crate::transport::Transport;
use crate::types::{ClaudeAgentOptions, Message};

pub async fn query(
    prompt: InputPrompt,
    options: Option<ClaudeAgentOptions>,
    transport: Option<Box<dyn Transport>>,
) -> Result<Vec<Message>> {
    let options = options.unwrap_or_default();
    let client = InternalClient::new();
    client.process_query(prompt, options, transport).await
}
