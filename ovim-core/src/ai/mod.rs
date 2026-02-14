pub mod chat_types;
mod config;
mod extract;
pub mod project_context;
pub mod prompt;
mod provider;
pub mod scope;
pub mod stream_parsers;
pub mod tools;
mod types;

pub use chat_types::{
    ChatFocus, ChatMessage, ChatOpts, ChatRole, ConversationTree, StreamChunk, ToolCallInfo,
};
pub use config::{default_api_key_env, infer_provider, parse_edit_format_str, parse_provider_str};
pub use config::{AiConfig, AiProfileConfig, ChatContextConfig, ProjectContextConfig};
pub use extract::{extract_response, AiExtractedResponse};
pub(crate) use provider::append_project_context;
pub(crate) use provider::resolve_chat_system_prompt;
pub use provider::{request_ai_edit, stream_ai_chat};
pub use scope::{Capabilities, RequiredScope, ScopeContext};
pub use tools::{ToolDefinition, ToolRegistry, ToolResult};
pub use types::{
    AgentLoopConfig, AiContextPack, AiJobResult, AiProviderKind, AiRequest, ApiKeyConfig,
    BufferLock, CodeSlice, ContextGatheringPolicy, DiagnosticFact, DiagnosticScope, EditFormat,
    FileScope, ProfileScope, RetryPolicy, SymbolFact, PROFILE_LOCAL,
};
