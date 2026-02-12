pub mod chat_types;
mod config;
mod extract;
mod provider;
pub mod stream_parsers;
mod types;

pub use chat_types::{ChatFocus, ChatMessage, ChatOpts, ChatRole, ConversationTree, StreamChunk};
pub use config::{default_api_key_env, infer_provider, parse_provider_str};
pub use config::{AiConfig, AiProfileConfig};
pub use extract::{extract_response, AiExtractedResponse};
pub use provider::{request_ai_edit, stream_ai_chat};
pub use types::{
    AgentMode, AiContextPack, AiJobResult, AiProviderKind, AiRequest, BufferLock, CapabilityTier,
    CodeSlice, ContextPolicy, DiagnosticFact, EditMode, ExtractionStrategy, FileScope,
    ProfileScope, SymbolFact, PROFILE_LOCAL,
};
