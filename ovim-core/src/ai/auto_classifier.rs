use super::auto_mode::{ClassifierRequest, ClassifierVerdict};
use crate::run_log::OperationId;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

pub type ClassifierFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

/// Provider-independent execution seam for auto-mode policy classifiers.
pub trait AutoModeClassifier: Send + Sync {
    fn classify<'a>(
        &'a self,
        request: &'a ClassifierRequest,
        operation_id: &'a OperationId,
    ) -> Pin<Box<dyn Future<Output = Result<ClassifierVerdict>> + Send + 'a>>;
}

/// Fully separated inputs to a structured classifier turn. The stable prefix
/// and schema must never contain chat history; `dynamic_payload` is the compact
/// projection produced by `ClassifierRequest`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassifierInvocation {
    pub stable_instructions: String,
    pub output_schema: Value,
    pub dynamic_payload: String,
    pub cwd: PathBuf,
    pub client_user_message_id: String,
}

pub trait ClassifierTransport: Send + Sync {
    fn invoke<'a>(&'a self, invocation: ClassifierInvocation) -> ClassifierFuture<'a>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassifierProfile {
    pub provider: &'static str,
    pub model: &'static str,
    pub reasoning_effort: &'static str,
    pub approval_policy: &'static str,
    pub sandbox: &'static str,
    pub dynamic_tools: bool,
    pub ephemeral_thread: bool,
}

#[derive(Default)]
pub struct CodexClassifierTransport;

impl ClassifierTransport for CodexClassifierTransport {
    fn invoke<'a>(&'a self, invocation: ClassifierInvocation) -> ClassifierFuture<'a> {
        Box::pin(async move {
            super::codex_app_server::request_auto_mode_classification(
                &invocation.stable_instructions,
                &invocation.output_schema,
                &invocation.dynamic_payload,
                &invocation.cwd,
                &invocation.client_user_message_id,
            )
            .await
        })
    }
}

pub struct CodexAutoModeClassifier<T = CodexClassifierTransport> {
    transport: T,
}

impl Default for CodexAutoModeClassifier {
    fn default() -> Self {
        Self {
            transport: CodexClassifierTransport,
        }
    }
}

impl<T> CodexAutoModeClassifier<T> {
    pub fn with_transport(transport: T) -> Self {
        Self { transport }
    }

    pub const fn profile() -> ClassifierProfile {
        ClassifierProfile {
            provider: "codex_subscription",
            model: super::codex_app_server::AUTO_MODE_CLASSIFIER_MODEL,
            reasoning_effort: super::codex_app_server::AUTO_MODE_CLASSIFIER_EFFORT,
            approval_policy: "never",
            sandbox: "read-only",
            dynamic_tools: false,
            ephemeral_thread: super::codex_app_server::AUTO_MODE_CLASSIFIER_EPHEMERAL_THREAD,
        }
    }

    fn prepare(
        request: &ClassifierRequest,
        operation_id: &OperationId,
    ) -> Result<ClassifierInvocation> {
        let (stable, dynamic_payload) = request
            .serialized_parts()
            .context("failed to serialize auto-mode classifier request")?;
        let (stable_instructions, schema_wrapper): (String, Value) =
            serde_json::from_str(&stable).context("invalid stable auto-mode classifier prefix")?;
        let output_schema = schema_wrapper
            .get("schema")
            .cloned()
            .ok_or_else(|| anyhow!("auto-mode classifier schema has no `schema` member"))?;
        Ok(ClassifierInvocation {
            stable_instructions,
            output_schema,
            dynamic_payload,
            cwd: request.dynamic.proposal.project_root.clone(),
            client_user_message_id: operation_id.as_str().to_string(),
        })
    }
}

impl<T: ClassifierTransport> AutoModeClassifier for CodexAutoModeClassifier<T> {
    fn classify<'a>(
        &'a self,
        request: &'a ClassifierRequest,
        operation_id: &'a OperationId,
    ) -> Pin<Box<dyn Future<Output = Result<ClassifierVerdict>> + Send + 'a>> {
        Box::pin(async move {
            let invocation = Self::prepare(request, operation_id)?;
            let raw = self
                .transport
                .invoke(invocation)
                .await
                .context("Codex auto-mode classifier failed")?;
            // Parsing is deliberately the only path to an Allow. Transport,
            // protocol, structured-output, and parse failures all return an
            // error to the caller, which must escalate rather than execute.
            ClassifierVerdict::parse_strict(&raw)
                .map_err(anyhow::Error::new)
                .context("Codex auto-mode classifier returned an invalid verdict")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::auto_mode::{
        ClassifierDecision, ConversationAuthorizationContext, ExplicitAuthorization, ShellProposal,
        AUTO_MODE_POLICY_VERSION,
    };
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockTransport {
        result: Result<String, String>,
        invocations: Arc<Mutex<Vec<ClassifierInvocation>>>,
    }

    impl MockTransport {
        fn returning(result: Result<String, String>) -> Self {
            Self {
                result,
                invocations: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl ClassifierTransport for MockTransport {
        fn invoke<'a>(&'a self, invocation: ClassifierInvocation) -> ClassifierFuture<'a> {
            self.invocations.lock().unwrap().push(invocation);
            let result = self.result.clone();
            Box::pin(async move { result.map_err(anyhow::Error::msg) })
        }
    }

    fn request(command: &str, instruction: &str) -> ClassifierRequest {
        ClassifierRequest::new(
            ShellProposal {
                command: command.into(),
                cwd: PathBuf::from("/project"),
                project_root: PathBuf::from("/project"),
                requested_capabilities: BTreeSet::new(),
            },
            ConversationAuthorizationContext {
                explicit_user_instructions: vec![ExplicitAuthorization {
                    instruction: instruction.into(),
                    project_root: PathBuf::from("/project"),
                    source_id: "turn-9".into(),
                }],
                authorized_objectives: Vec::new(),
            },
        )
    }

    fn valid_verdict(decision: &str) -> String {
        format!(
            r#"{{"policy_version":"{AUTO_MODE_POLICY_VERSION}","decision":"{decision}","scope":{{"project_root":"/project"}},"reason":"within explicit scope","confidence":0.91,"expiry":{{"kind":"after_command"}}}}"#
        )
    }

    #[tokio::test]
    async fn separates_stable_prefix_from_compact_dynamic_payload() {
        let transport = MockTransport::returning(Ok(valid_verdict("ask")));
        let seen = transport.invocations.clone();
        let classifier = CodexAutoModeClassifier::with_transport(transport);
        let first_request = request("./deploy prod", "deploy this to prod");
        let operation = OperationId::parse("op_classify_9").unwrap();

        classifier
            .classify(&first_request, &operation)
            .await
            .unwrap();
        let second_request = request("cargo test", "run the tests");
        let second_operation = OperationId::parse("op_classify_10").unwrap();
        classifier
            .classify(&second_request, &second_operation)
            .await
            .unwrap();
        let invocations = seen.lock().unwrap();
        let invocation = &invocations[0];
        let second = &invocations[1];
        assert_eq!(invocation.client_user_message_id, operation.as_str());
        assert_eq!(invocation.cwd, PathBuf::from("/project"));
        assert!(!invocation
            .stable_instructions
            .contains("deploy this to prod"));
        assert!(!invocation.stable_instructions.contains("./deploy prod"));
        assert!(invocation.dynamic_payload.contains("deploy this to prod"));
        assert!(invocation.dynamic_payload.contains("./deploy prod"));
        assert_eq!(invocation.output_schema["additionalProperties"], false);
        assert_eq!(
            invocation.output_schema["properties"]["policy_version"]["const"],
            AUTO_MODE_POLICY_VERSION
        );
        assert_eq!(invocation.stable_instructions, second.stable_instructions);
        assert_eq!(invocation.output_schema, second.output_schema);
        assert_ne!(invocation.dynamic_payload, second.dynamic_payload);
        assert!(!second.dynamic_payload.contains("deploy this to prod"));
        assert!(!second.dynamic_payload.contains("./deploy prod"));
    }

    #[tokio::test]
    async fn accepts_only_strict_verdict_json() {
        let classifier = CodexAutoModeClassifier::with_transport(MockTransport::returning(Ok(
            valid_verdict("allow"),
        )));
        let operation = OperationId::parse("op_classify_strict").unwrap();
        let verdict = classifier
            .classify(&request("cargo test", "test it"), &operation)
            .await
            .unwrap();
        assert_eq!(verdict.decision, ClassifierDecision::Allow);

        let fenced = CodexAutoModeClassifier::with_transport(MockTransport::returning(Ok(
            format!("```json\n{}\n```", valid_verdict("allow")),
        )));
        assert!(fenced
            .classify(&request("cargo test", "test it"), &operation)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn transport_and_parse_failures_never_produce_allow() {
        let operation = OperationId::parse("op_classify_failure").unwrap();
        let transport_failure = CodexAutoModeClassifier::with_transport(MockTransport::returning(
            Err("app-server unavailable".into()),
        ));
        assert!(transport_failure
            .classify(&request("rm -rf build", "clean build output"), &operation)
            .await
            .is_err());

        let malformed = CodexAutoModeClassifier::with_transport(MockTransport::returning(Ok(
            r#"{"decision":"allow"}"#.into(),
        )));
        assert!(malformed
            .classify(&request("rm -rf build", "clean build output"), &operation)
            .await
            .is_err());
    }

    #[test]
    fn codex_profile_is_subscription_luna_low_and_toolless() {
        let profile = CodexAutoModeClassifier::<CodexClassifierTransport>::profile();
        assert_eq!(profile.provider, "codex_subscription");
        assert_eq!(profile.model, "gpt-5.6-luna");
        assert_eq!(profile.reasoning_effort, "low");
        assert_eq!(profile.approval_policy, "never");
        assert_eq!(profile.sandbox, "read-only");
        assert!(!profile.dynamic_tools);
        assert!(profile.ephemeral_thread);
    }
}
