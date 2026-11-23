//! Code-lens hit targets derived from the final rows, after wrapping, clipping,
//! horizontal scrolling and diagnostic placement have all been applied.

use super::decorations::decoration_to_ratatui_style;
use super::rows::RowOutput;
use crate::display::grapheme_display_width;
use crate::editor::Editor;
use ovim_core::editor::decoration::DecorationSource;
use ovim_core::editor::CodeLensHitTarget;
use ratatui::layout::Rect;
use unicode_segmentation::UnicodeSegmentation;

fn width(text: &str) -> usize {
    text.graphemes(true).map(grapheme_display_width).sum()
}

pub(super) fn visible_targets(
    editor: &Editor,
    rows: &RowOutput,
    area: Rect,
) -> Vec<CodeLensHitTarget> {
    let mut targets = Vec::new();
    let Some(file_path) = editor.buffer().file_path() else {
        return targets;
    };
    for (row_index, (row, line)) in rows.lines.iter().zip(&rows.logical_lines).enumerate() {
        let Some(line) = line else { continue };
        let actions = editor.code_lens_actions(*line);
        if actions.is_empty() {
            continue;
        }
        let decorations = editor.decorations.eol_for_line(*line);
        // Only the first EOL decoration is drawn by the terminal renderer.
        let Some(decoration) = decorations
            .first()
            .filter(|dec| dec.source == DecorationSource::CodeLens)
        else {
            continue;
        };
        let style = decoration_to_ratatui_style(&decoration.style);
        let mut column = 0;
        for span in &row.spans {
            let visible = if span.content.as_ref() == decoration.text {
                span.content.as_ref()
            } else {
                // Exclude the renderer's ellipsis from clipped hit regions,
                // while preserving literal dots in a fully displayed title.
                span.content.strip_suffix("...").unwrap_or(&span.content)
            };
            if span.style == style && decoration.text.starts_with(visible) && !visible.is_empty() {
                let mut offset = 2; // the decoration's leading gap
                for action in &actions {
                    let end = offset + width(&action.title);
                    if end <= width(visible) && column + end <= area.width as usize {
                        targets.push(CodeLensHitTarget {
                            area: ovim_core::Rect {
                                x: area.x + (column + offset) as u16,
                                y: area.y + row_index as u16,
                                width: (end - offset) as u16,
                                height: 1,
                            },
                            file_path: file_path.into(),
                            version: editor.buffer().version(),
                            index: action.index,
                            generation: editor.code_lens_generation(),
                            mode: action.mode,
                        });
                    }
                    offset = end + 3; // " │ "
                }
            }
            column += width(&span.content);
        }
    }
    targets
}
