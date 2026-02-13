pub mod chat_types;
mod config;
mod extract;
mod provider;
pub mod scope;
pub mod stream_parsers;
pub mod tools;
mod types;

pub use chat_types::{
    ChatFocus, ChatMessage, ChatOpts, ChatRole, ConversationTree, StreamChunk, ToolCallInfo,
};
pub use config::{default_api_key_env, infer_provider, parse_provider_str};
pub use config::{AiConfig, AiProfileConfig};
pub use extract::{extract_response, AiExtractedResponse};
pub use provider::{request_ai_edit, stream_ai_chat};
pub use scope::{Capabilities, RequiredScope, ScopeContext};
pub use tools::{ToolDefinition, ToolRegistry, ToolResult};
pub use types::{
    AgentMode, AiContextPack, AiJobResult, AiProviderKind, AiRequest, BufferLock, CapabilityTier,
    CodeSlice, ContextPolicy, DiagnosticFact, EditMode, ExtractionStrategy, FileScope,
    ProfileScope, SymbolFact, PROFILE_LOCAL,
};
