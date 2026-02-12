pub mod chat_types;
mod config;
mod extract;
mod provider;
mod types;

pub use chat_types::{ChatFocus, ChatMessage, ChatOpts, ChatRole, ConversationTree};
pub use config::{AiConfig, AiProfileConfig};
pub use extract::{extract_response, AiExtractedResponse};
pub use provider::{request_ai_chat, request_ai_edit};
pub use types::{
    AgentMode, AiContextPack, AiJobResult, AiProviderKind, AiRequest, BufferLock, CapabilityTier,
    CodeSlice, ContextPolicy, DiagnosticFact, ExtractionStrategy, SymbolFact, PROFILE_LOCAL,
};
