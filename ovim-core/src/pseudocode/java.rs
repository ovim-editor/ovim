use super::{parse, preserve, remove, replace, Edit};
use tree_sitter::Node;

pub(super) fn edits(source: &str, base: usize, edits: &mut Vec<Edit>) -> anyhow::Result<()> {
    let tree = parse(source, tree_sitter_java::LANGUAGE.into())?;
    visit(tree.root_node(), source, base, edits);
    Ok(())
}

fn visit(node: Node<'_>, source: &str, base: usize, edits: &mut Vec<Edit>) {
    if node.is_error() {
        preserve(edits, base + node.start_byte()..base + node.end_byte());
        return;
    }
    if node.is_missing() {
        return;
    }
    let kind = node.kind();
    if matches!(
        kind,
        "string_literal" | "character_literal" | "line_comment" | "block_comment"
    ) {
        preserve(edits, base + node.start_byte()..base + node.end_byte());
        return;
    }
    if !node.has_error() {
        match kind {
            "package_declaration"
            | "import_declaration"
            | "annotation"
            | "marker_annotation"
            | "type_parameters"
            | "type_arguments"
            | "throws"
            | "superclass"
            | "super_interfaces"
            | "permits" => {
                let start = if matches!(
                    kind,
                    "throws" | "superclass" | "super_interfaces" | "permits"
                ) {
                    source[..node.start_byte()]
                        .trim_end_matches([' ', '\t'])
                        .len()
                } else {
                    node.start_byte()
                };
                remove(source, start..node.end_byte(), base, edits);
                return;
            }
            "public" | "private" | "protected" | "static" | "final" | "abstract" | "native"
            | "strictfp" | "default"
                if node.parent().is_some_and(|p| p.kind() == "modifiers") =>
            {
                remove(source, node.byte_range(), base, edits);
                return;
            }
            "method_declaration"
            | "field_declaration"
            | "local_variable_declaration"
            | "formal_parameter"
            | "enhanced_for_statement"
            | "resource" => {
                if let Some(ty) = node.child_by_field_name("type") {
                    remove(source, ty.byte_range(), base, edits);
                }
                if let Some(dimensions) = node.child_by_field_name("dimensions") {
                    remove(source, dimensions.byte_range(), base, edits);
                }
            }
            "catch_type" => {
                remove(source, node.byte_range(), base, edits);
                return;
            }
            "spread_parameter" => {
                let mut cursor = node.walk();
                if let Some(name) = node
                    .named_children(&mut cursor)
                    .find(|n| n.kind() == "variable_declarator")
                {
                    remove(source, node.start_byte()..name.start_byte(), base, edits);
                }
            }
            "variable_declarator" => {
                if let Some(dimensions) = node.child_by_field_name("dimensions") {
                    remove(source, dimensions.byte_range(), base, edits);
                }
            }
            "cast_expression" => {
                if let Some(value) = node.child_by_field_name("value") {
                    remove(source, node.start_byte()..value.start_byte(), base, edits);
                }
            }
            "new"
                if node
                    .parent()
                    .is_some_and(|p| p.kind() == "object_creation_expression") =>
            {
                remove(source, node.byte_range(), base, edits);
            }
            "{" | "}"
                if node.parent().is_some_and(|p| {
                    matches!(
                        p.kind(),
                        "block"
                            | "class_body"
                            | "constructor_body"
                            | "interface_body"
                            | "enum_body"
                            | "switch_block"
                    )
                }) =>
            {
                // Empty blocks retain their braces; otherwise they could look
                // like a missing body, or an empty loop could disappear.
                let parent = node.parent().unwrap();
                // Inline groups need delimiters: indentation cannot tell where
                // an inline conditional ends and the following statement begins.
                if parent.start_position().row == parent.end_position().row {
                    return;
                }
                let inside = source[parent.byte_range()].trim();
                if inside
                    .strip_prefix('{')
                    .and_then(|s| s.strip_suffix('}'))
                    .is_some_and(|s| s.trim().is_empty())
                {
                    return;
                }
                let before = &source[..node.start_byte()];
                let line_start = before.rfind('\n').map_or(0, |n| n + 1);
                let only_indent = source[line_start..node.start_byte()].trim().is_empty();
                if kind == "{" && !only_indent {
                    let start = before.trim_end_matches([' ', '\t']).len();
                    let replacement = if source[node.end_byte()..]
                        .chars()
                        .next()
                        .is_some_and(|c| !c.is_whitespace() && c != '}')
                    {
                        ": "
                    } else {
                        ":"
                    };
                    replace(edits, start + base..node.end_byte() + base, replacement);
                } else {
                    remove(source, node.byte_range(), base, edits);
                }
            }
            "array_creation_expression" => {
                if let Some(value) = node.child_by_field_name("value") {
                    remove(source, node.start_byte()..value.start_byte(), base, edits);
                } else if let Some(dimensions) = node.child_by_field_name("dimensions") {
                    replace(
                        edits,
                        base + node.start_byte()..base + dimensions.start_byte(),
                        "array",
                    );
                }
            }
            ";" if node.parent().is_some_and(|p| {
                p.kind() == "empty_statement"
                    || ["body", "consequence", "alternative"].iter().any(|field| {
                        p.child_by_field_name(field)
                            .is_some_and(|body| body.id() == node.id())
                    })
            }) =>
            {
                let start = source[..node.start_byte()]
                    .trim_end_matches([' ', '\t'])
                    .len();
                replace(edits, base + start..base + node.end_byte(), " {}");
            }
            ";" => {
                let tail = source[node.end_byte()..].split('\n').next().unwrap_or("");
                if tail.trim_matches([' ', '\t', '\r', '}']).is_empty() && !is_for_separator(node) {
                    replace(edits, node.start_byte() + base..node.end_byte() + base, "");
                }
            }
            "(" | ")"
                if node.parent().is_some_and(|p| {
                    matches!(p.kind(), "enhanced_for_statement" | "for_statement")
                        || (p.kind() == "parenthesized_expression"
                            && p.parent().is_some_and(|g| {
                                matches!(
                                    g.kind(),
                                    "if_statement"
                                        | "while_statement"
                                        | "switch_expression"
                                        | "synchronized_statement"
                                )
                            }))
                }) =>
            {
                let replacement = if kind == "("
                    && source[..node.start_byte()]
                        .chars()
                        .next_back()
                        .is_some_and(|c| !c.is_whitespace())
                {
                    " "
                } else {
                    ""
                };
                replace(
                    edits,
                    node.start_byte() + base..node.end_byte() + base,
                    replacement,
                )
            }
            ":" if node
                .parent()
                .is_some_and(|p| p.kind() == "enhanced_for_statement") =>
            {
                let start = source[..node.start_byte()]
                    .trim_end_matches([' ', '\t'])
                    .len();
                let end = node.end_byte()
                    + source[node.end_byte()..]
                        .bytes()
                        .take_while(|b| matches!(b, b' ' | b'\t'))
                        .count();
                replace(edits, start + base..end + base, " in ");
            }
            _ => {}
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(child, source, base, edits);
    }
}

fn is_for_separator(node: Node<'_>) -> bool {
    let mut parent = node.parent();
    while let Some(ancestor) = parent {
        if ancestor.kind() == "for_statement" {
            return ancestor
                .child_by_field_name("body")
                .is_some_and(|body| node.end_byte() <= body.start_byte());
        }
        if matches!(ancestor.kind(), "block" | "class_body" | "program") {
            break;
        }
        parent = ancestor.parent();
    }
    false
}
