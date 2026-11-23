use super::{java, parse, preserve, remove, replace, Edit};
use tree_sitter::Node;

pub(super) fn edits(source: &str, edits: &mut Vec<Edit>, formatted: bool) -> anyhow::Result<()> {
    let tree = parse(source, tree_sitter_md::LANGUAGE.into())?;
    blocks(tree.root_node(), source, edits, formatted)
}

fn blocks(
    node: Node<'_>,
    source: &str,
    edits: &mut Vec<Edit>,
    formatted: bool,
) -> anyhow::Result<()> {
    if formatted
        && matches!(
            node.kind(),
            "paragraph" | "inline" | "link_reference_definition"
        )
    {
        preserve(edits, node.byte_range());
    }
    match node.kind() {
        "fenced_code_block" => {
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            let language = children
                .iter()
                .find(|n| n.kind() == "info_string")
                .map(|n| {
                    source[n.byte_range()]
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                });
            // Other languages remain verbatim, including their fences.
            if language.is_some_and(|lang| lang.eq_ignore_ascii_case("java")) {
                for child in children {
                    match child.kind() {
                        "code_fence_content" => {
                            java::edits(&source[child.byte_range()], child.start_byte(), edits)?
                        }
                        "fenced_code_block_delimiter" | "info_string" if !formatted => {
                            remove(source, child.byte_range(), 0, edits)
                        }
                        _ => {}
                    }
                }
            } else {
                preserve(edits, node.byte_range());
            }
            return Ok(());
        }
        "indented_code_block" | "html_block" => {
            preserve(edits, node.byte_range());
            return Ok(());
        }
        "inline" if !formatted => {
            let text = &source[node.byte_range()];
            let tree = parse(text, tree_sitter_md::INLINE_LANGUAGE.into())?;
            inline(tree.root_node(), node.start_byte(), edits);
            return Ok(());
        }
        "atx_h1_marker"
        | "atx_h2_marker"
        | "atx_h3_marker"
        | "atx_h4_marker"
        | "atx_h5_marker"
        | "atx_h6_marker"
        | "setext_h1_underline"
        | "setext_h2_underline"
        | "link_reference_definition"
            if !formatted =>
        {
            remove(source, node.byte_range(), 0, edits);
            return Ok(());
        }
        // Keep list, quote, table, and task markers: they convey structure.
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        blocks(child, source, edits, formatted)?;
    }
    Ok(())
}

fn inline(node: Node<'_>, base: usize, edits: &mut Vec<Edit>) {
    match node.kind() {
        "code_span" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "code_span_delimiter" {
                    replace(
                        edits,
                        base + child.start_byte()..base + child.end_byte(),
                        "",
                    );
                }
            }
            return;
        }
        "emphasis_delimiter" => {
            replace(edits, base + node.start_byte()..base + node.end_byte(), "")
        }
        "inline_link" | "full_reference_link" | "collapsed_reference_link" | "image" => {
            let mut cursor = node.walk();
            if let Some(label) = node
                .children(&mut cursor)
                .find(|n| matches!(n.kind(), "link_text" | "image_description"))
            {
                replace(
                    edits,
                    base + node.start_byte()..base + label.start_byte(),
                    if node.kind() == "image" {
                        "Image: "
                    } else {
                        ""
                    },
                );
                replace(edits, base + label.end_byte()..base + node.end_byte(), "");
                inline(label, base, edits);
                return;
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        inline(child, base, edits);
    }
}

/// Inline styles are absent from the block grammar's normal syntax query.
/// Capture them separately, then use the shared byte map to conceal delimiters.
pub(super) fn styles(
    source: &str,
) -> anyhow::Result<Vec<(std::ops::Range<usize>, crate::syntax::HighlightGroup)>> {
    fn visit(
        node: Node<'_>,
        base: usize,
        output: &mut Vec<(std::ops::Range<usize>, crate::syntax::HighlightGroup)>,
    ) {
        use crate::syntax::HighlightGroup;
        let group = match node.kind() {
            "strong_emphasis" => Some(HighlightGroup::MarkupBold),
            "emphasis" => Some(HighlightGroup::MarkupItalic),
            "code_span" => Some(HighlightGroup::MarkupRaw),
            _ => None,
        };
        if let Some(group) = group {
            output.push((base + node.start_byte()..base + node.end_byte(), group));
        }
        if node.kind() != "code_span" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                visit(child, base, output);
            }
        }
    }
    fn blocks(
        node: Node<'_>,
        source: &str,
        output: &mut Vec<(std::ops::Range<usize>, crate::syntax::HighlightGroup)>,
    ) -> anyhow::Result<()> {
        if node.kind() == "inline" {
            let tree = parse(
                &source[node.byte_range()],
                tree_sitter_md::INLINE_LANGUAGE.into(),
            )?;
            visit(tree.root_node(), node.start_byte(), output);
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                blocks(child, source, output)?;
            }
        }
        Ok(())
    }
    let tree = parse(source, tree_sitter_md::LANGUAGE.into())?;
    let mut output = Vec::new();
    blocks(tree.root_node(), source, &mut output)?;
    Ok(output)
}
