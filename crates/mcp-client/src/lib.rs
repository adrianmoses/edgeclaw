pub mod client;
pub mod protocol;

pub use client::{McpClient, ServerCapabilities};
pub use protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
