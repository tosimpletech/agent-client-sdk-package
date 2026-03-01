use async_trait::async_trait;
use serde_json::Value;

use crate::errors::Result;

pub mod subprocess_cli;

#[async_trait]
pub trait Transport: Send {
    async fn connect(&mut self) -> Result<()>;
    async fn write(&mut self, data: &str) -> Result<()>;
    async fn end_input(&mut self) -> Result<()>;
    async fn read_next_message(&mut self) -> Result<Option<Value>>;
    async fn close(&mut self) -> Result<()>;
    fn is_ready(&self) -> bool;
}

