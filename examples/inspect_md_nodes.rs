// Inspect tree-sitter-md grammar nodes
// Run with: cargo run --example inspect_md_nodes

fn main() {
    use tree_sitter::Parser;

    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_md::LANGUAGE.into();
    parser.set_language(&language).unwrap();

    let source = r#"# Heading 1

This is a paragraph with **bold** and *italic* text.

## Heading 2

```rust
fn main() {
    println!("Hello");
}
```

- List item 1
- List item 2

`inline code`
"#;

    let tree = parser.parse(source, None).unwrap();

    fn print_tree(node: tree_sitter::Node, source: &str, depth: usize) {
        let indent = "  ".repeat(depth);
        let text: String = source[node.byte_range()].chars().take(40).collect();
        let text = text.replace('\n', "\\n");
        println!(
            "{}[{}] {}..{} = {:?}",
            indent,
            node.kind(),
            node.byte_range().start,
            node.byte_range().end,
            text
        );
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                print_tree(child, source, depth + 1);
            }
        }
    }

    println!("Tree-sitter-md grammar nodes:\n");
    print_tree(tree.root_node(), source, 0);
}
