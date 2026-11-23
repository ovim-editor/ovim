//! LSP signature help (parameter hints while typing a call).
//!
//! Triggered from insert mode by `(` and `,` (and retriggered on every edit
//! while the popup is open, so the active parameter follows the cursor). The
//! server's `SignatureHelp` is reduced to a [`SignatureHelpState`] that both
//! frontends render as a small popup above the cursor line.

use super::super::Editor;
use crate::editor::lsp_slot::SignatureHelpResult;
use crate::editor::lsp_state::SignatureHelpState;
use crate::lsp::uri_from_file_path;
use anyhow::{anyhow, Result};

impl SignatureHelpState {
    /// Splits the label into `(before, active, after)` around the active
    /// parameter for highlighting. Without an active parameter everything is
    /// `before`.
    pub fn label_segments(&self) -> (String, String, String) {
        let chars: Vec<char> = self.label.chars().collect();
        match self.active_param {
            Some((start, end)) if start < end && end <= chars.len() => (
                chars[..start].iter().collect(),
                chars[start..end].iter().collect(),
                chars[end..].iter().collect(),
            ),
            _ => (self.label.clone(), String::new(), String::new()),
        }
    }

    /// Builds the display model from a server response. Returns `None` when
    /// the server offered no signature.
    pub fn from_lsp(help: &lsp_types::SignatureHelp, anchor: (usize, usize)) -> Option<Self> {
        if help.signatures.is_empty() {
            return None;
        }
        let signature_index = (help.active_signature.unwrap_or(0) as usize)
            .min(help.signatures.len().saturating_sub(1));
        let signature = &help.signatures[signature_index];
        let params = signature.parameters.as_deref().unwrap_or(&[]);

        // Per-signature activeParameter overrides the response-level one; an
        // absent or out-of-range value means "the first parameter" (LSP 3.17).
        let requested = signature
            .active_parameter
            .or(help.active_parameter)
            .map(|index| index as usize);
        let active_param_index = if params.is_empty() {
            None
        } else {
            Some(requested.filter(|index| *index < params.len()).unwrap_or(0))
        };
        let active = active_param_index.and_then(|index| params.get(index));
        let active_param = active.and_then(|param| parameter_char_range(&signature.label, param));

        Some(Self {
            label: signature.label.clone(),
            active_param,
            active_param_index,
            signature_index,
            signature_count: help.signatures.len(),
            documentation: signature
                .documentation
                .as_ref()
                .and_then(documentation_text),
            parameter_documentation: active
                .and_then(|param| param.documentation.as_ref())
                .and_then(documentation_text),
            anchor,
        })
    }
}

fn documentation_text(doc: &lsp_types::Documentation) -> Option<String> {
    let text = match doc {
        lsp_types::Documentation::String(text) => text.clone(),
        lsp_types::Documentation::MarkupContent(markup) => markup.value.clone(),
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Char range of a parameter inside the signature label.
fn parameter_char_range(
    label: &str,
    param: &lsp_types::ParameterInformation,
) -> Option<(usize, usize)> {
    match &param.label {
        lsp_types::ParameterLabel::LabelOffsets([start, end]) => {
            // Offsets are UTF-16 code units into the label.
            let mut utf16 = 0u32;
            let mut start_char = None;
            let mut end_char = None;
            for (char_index, ch) in label.chars().enumerate() {
                if utf16 == *start {
                    start_char = Some(char_index);
                }
                if utf16 == *end {
                    end_char = Some(char_index);
                    break;
                }
                utf16 += ch.len_utf16() as u32;
            }
            let total = label.chars().count();
            let start_char = start_char.or((utf16 == *start).then_some(total))?;
            let end_char = end_char.unwrap_or(total);
            (start_char < end_char).then_some((start_char, end_char))
        }
        lsp_types::ParameterLabel::Simple(text) => {
            if text.is_empty() {
                return None;
            }
            // Skip the function name/receiver: parameters live after the
            // first `(`.
            let search_from = label.find('(').map(|index| index + 1).unwrap_or(0);
            let byte_start = label[search_from..].find(text.as_str())? + search_from;
            let start = label[..byte_start].chars().count();
            Some((start, start + text.chars().count()))
        }
    }
}

/// Words that put a `(` in front of a condition, not an argument list.
const NON_CALL_KEYWORDS: &[&str] = &[
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "synchronized",
    "return",
    "else",
    "match",
    "with",
    "await",
    "in",
    "and",
    "or",
    "not",
    "using",
    "lock",
    "foreach",
    "when",
];

/// Whether `before_cursor` (the text up to the cursor, possibly several lines)
/// ends inside the argument list of a call: an unclosed `(` whose left
/// neighbour is an identifier, `>` (generic call) or `]`. Balanced groups are
/// skipped; a `;` or brace at nesting level 0 means the statement ended.
pub(crate) fn text_ends_inside_call(before_cursor: &str) -> bool {
    let chars: Vec<char> = before_cursor.chars().collect();
    let mut depth = 0usize;
    let mut index = chars.len();
    while index > 0 {
        index -= 1;
        match chars[index] {
            ')' => depth += 1,
            '(' if depth > 0 => depth -= 1,
            '(' => {
                let mut left = index;
                while left > 0 && chars[left - 1].is_whitespace() {
                    left -= 1;
                }
                let is_call = match left.checked_sub(1).map(|i| chars[i]) {
                    Some(c) if c.is_alphanumeric() || c == '_' || c == '$' => {
                        let word: String = chars[..left]
                            .iter()
                            .rev()
                            .take_while(|c| c.is_alphanumeric() || **c == '_' || **c == '$')
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();
                        !NON_CALL_KEYWORDS.contains(&word.as_str())
                    }
                    Some('>') | Some(']') => true,
                    _ => false,
                };
                if is_call {
                    return true;
                }
                // A grouping paren: an enclosing call may still be open.
            }
            ';' | '{' | '}' if depth == 0 => return false,
            _ => {}
        }
    }
    false
}

impl Editor {
    /// Whether the cursor sits inside the argument list of an unfinished call
    /// (looks back over at most 20 lines).
    pub(crate) fn cursor_in_unfinished_call(&self) -> bool {
        let cursor = self.buffer().cursor();
        let line = cursor.line();
        let first = line.saturating_sub(20);
        let rope = self.buffer().rope();
        let start = rope.line_to_char(first);
        let end = rope.line_to_char(line) + self.buffer().cursor_char_col().0;
        text_ends_inside_call(&rope.slice(start..end.max(start)).to_string())
    }

    /// Insert mode: shows the signature popup when the cursor is (back) inside
    /// an unfinished call, like VS Code after moving into the arguments or
    /// accepting a method completion. Does nothing while the popup is already
    /// open (edits re-ask on their own).
    pub(crate) fn request_signature_help_if_in_call(&mut self) {
        if self.mode() != crate::mode::Mode::Insert
            || self.signature_help_active()
            || !self.cursor_in_unfinished_call()
        {
            return;
        }
        self.request_signature_help();
    }

    /// Ask the server for signature help at the cursor.
    pub fn request_signature_help(&mut self) {
        self.lsp.intents.signature_help = true;
    }

    /// The signature help popup content, when visible.
    pub fn signature_help(&self) -> Option<&SignatureHelpState> {
        self.lsp.state.signature_help.as_deref()
    }

    /// Shows `help` as the popup, anchored at the cursor — what an answer
    /// from the server turns into. Returns false when it offered no signature.
    pub fn show_signature_help(&mut self, help: &lsp_types::SignatureHelp) -> bool {
        let cursor = self.buffer().cursor();
        let anchor = (cursor.line(), cursor.col().0);
        let state = SignatureHelpState::from_lsp(help, anchor).map(Box::new);
        let shown = state.is_some();
        self.lsp.state.signature_help = state;
        self.mark_dirty();
        shown
    }

    /// Dismiss the popup and abandon any in-flight request.
    pub fn clear_signature_help(&mut self) {
        if self.lsp.state.signature_help.take().is_some() {
            self.mark_dirty();
        }
        self.lsp.intents.signature_help = false;
        self.lsp.slots.signature_help.cancel();
    }

    /// Whether the popup is open (used to retrigger on every insert edit).
    pub fn signature_help_active(&self) -> bool {
        self.lsp.state.signature_help.is_some() || self.lsp.slots.signature_help.is_pending()
    }

    pub(in crate::editor) async fn signature_help_impl(&mut self) -> Result<bool> {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return Ok(false);
        };
        let Some(file_path) = self.buffer().file_path().map(|s| s.to_string()) else {
            return Ok(false);
        };
        let Some(language_id) = self.language_id_for_path(&file_path) else {
            return Ok(false);
        };
        let uri = uri_from_file_path(&file_path).ok_or_else(|| anyhow!("Invalid file path"))?;
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col().0);

        self.ensure_lsp_document_synced().await;

        let (tx, rx) = tokio::sync::oneshot::channel();
        let file_for_result = file_path.clone();
        let task = tokio::spawn(async move {
            let result = lsp
                .signature_help(&uri, line, character, &language_id)
                .await;
            let _ = tx.send(result.map(|help| SignatureHelpResult {
                help,
                file_path: file_for_result,
            }));
        });
        self.lsp.slots.signature_help.fire(task, rx);
        Ok(true)
    }

    /// Poll the signature help slot; returns true when the UI changed.
    pub(in crate::editor) fn poll_signature_help_slot(&mut self) -> bool {
        let timeout = std::time::Duration::from_secs(5);
        let Some(result) = self.lsp.slots.signature_help.poll_with_timeout(timeout) else {
            return false;
        };
        // Late answers must not resurrect the popup after the user left
        // insert mode or switched files.
        let insert_active = self.mode == crate::mode::Mode::Insert;
        let same_file =
            matches!(&result, Ok(r) if self.buffer().file_path() == Some(r.file_path.as_str()));
        if !insert_active || !same_file {
            return self.lsp.state.signature_help.take().is_some();
        }
        let cursor = self.buffer().cursor();
        let anchor = (cursor.line(), cursor.col().0);
        let next = match result {
            Ok(r) => r
                .help
                .as_ref()
                .and_then(|help| SignatureHelpState::from_lsp(help, anchor))
                .map(Box::new),
            Err(_) => None,
        };
        if next == self.lsp.state.signature_help {
            return false;
        }
        self.lsp.state.signature_help = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{
        Documentation, ParameterInformation, ParameterLabel, SignatureHelp, SignatureInformation,
    };

    fn param(label: ParameterLabel) -> ParameterInformation {
        ParameterInformation {
            label,
            documentation: None,
        }
    }

    fn help(label: &str, params: Vec<ParameterInformation>, active: Option<u32>) -> SignatureHelp {
        SignatureHelp {
            signatures: vec![SignatureInformation {
                label: label.into(),
                documentation: None,
                parameters: Some(params),
                active_parameter: None,
            }],
            active_signature: Some(0),
            active_parameter: active,
        }
    }

    #[test]
    fn string_labels_are_located_after_the_open_paren() {
        // `a` appears in the method name; the parameter `a` must be found in
        // the parameter list.
        let help = help(
            "add(int a, int b)",
            vec![
                param(ParameterLabel::Simple("int a".into())),
                param(ParameterLabel::Simple("int b".into())),
            ],
            Some(1),
        );
        let state = SignatureHelpState::from_lsp(&help, (0, 0)).unwrap();
        assert_eq!(state.active_param, Some((11, 16)));
        let (before, active, after) = state.label_segments();
        assert_eq!(
            (before.as_str(), active.as_str(), after.as_str()),
            ("add(int a, ", "int b", ")")
        );
    }

    #[test]
    fn offset_labels_are_utf16_and_converted_to_chars() {
        // "é" is 1 UTF-16 unit, "😀" is 2: offsets and char indexes diverge.
        let label = "f(😀 x, int y)";
        let help = help(
            label,
            vec![
                param(ParameterLabel::LabelOffsets([2, 6])),
                param(ParameterLabel::LabelOffsets([8, 13])),
            ],
            Some(0),
        );
        let state = SignatureHelpState::from_lsp(&help, (0, 0)).unwrap();
        let (_, active, _) = state.label_segments();
        assert_eq!(active, "😀 x");
    }

    #[test]
    fn active_parameter_defaults_to_first_and_clamps() {
        let mk = |active| {
            help(
                "f(a, b)",
                vec![
                    param(ParameterLabel::Simple("a".into())),
                    param(ParameterLabel::Simple("b".into())),
                ],
                active,
            )
        };
        assert_eq!(
            SignatureHelpState::from_lsp(&mk(None), (0, 0))
                .unwrap()
                .active_param_index,
            Some(0)
        );
        assert_eq!(
            SignatureHelpState::from_lsp(&mk(Some(9)), (0, 0))
                .unwrap()
                .active_param_index,
            Some(0)
        );
        assert_eq!(
            SignatureHelpState::from_lsp(&mk(Some(1)), (0, 0))
                .unwrap()
                .active_param_index,
            Some(1)
        );
    }

    #[test]
    fn per_signature_active_parameter_wins_and_docs_are_kept() {
        let mut h = help(
            "f(a, b)",
            vec![
                param(ParameterLabel::Simple("a".into())),
                ParameterInformation {
                    label: ParameterLabel::Simple("b".into()),
                    documentation: Some(Documentation::String("the b".into())),
                },
            ],
            Some(0),
        );
        h.signatures[0].active_parameter = Some(1);
        h.signatures.push(h.signatures[0].clone());
        h.active_signature = Some(1);
        let state = SignatureHelpState::from_lsp(&h, (3, 4)).unwrap();
        assert_eq!(state.active_param_index, Some(1));
        assert_eq!(state.signature_index, 1);
        assert_eq!(state.signature_count, 2);
        assert_eq!(state.parameter_documentation.as_deref(), Some("the b"));
        assert_eq!(state.anchor, (3, 4));
    }

    #[test]
    fn empty_signatures_yield_no_popup() {
        let empty = SignatureHelp {
            signatures: vec![],
            active_signature: None,
            active_parameter: None,
        };
        assert!(SignatureHelpState::from_lsp(&empty, (0, 0)).is_none());
    }
}
