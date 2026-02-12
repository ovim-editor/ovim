mod config;
mod extract;
mod provider;
mod types;

pub use config::{AiConfig, AiProfileConfig};
pub use extract::{extract_response, AiExtractedResponse};
pub use provider::request_ai_edit;
pub use types::{
    AiJobResult, AiProviderKind, AiRequest, BufferLock, ExtractionStrategy, PROFILE_LOCAL,
};
