use crate::editor::Editor;
use ovim_core::editor::FileTreeAction;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

/// Renders the file tree explorer
pub fn render_file_tree(frame: &mut Frame, editor: &Editor, area: Rect) {
    if !editor.file_tree().is_visible() {
        return;
    }

    let tree = editor.file_tree();
    let flattened = tree.flattened();
    let selected_index = tree.selected_index();
    let scroll_offset = tree.scroll_offset();
    let action = tree.pending_action();

    // Reserve 1 row at the bottom for prompt when active
    let has_prompt = !matches!(action, FileTreeAction::None);
    let tree_area = if has_prompt && area.height > 2 {
        Rect::new(area.x, area.y, area.width, area.height - 1)
    } else {
        area
    };

    // Calculate viewport height (area height minus border rows)
    let viewport_height = tree_area.height.saturating_sub(1) as usize;

    // Create list items from the visible portion of the flattened tree
    let items: Vec<ListItem> = flattened
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(viewport_height)
        .map(|(idx, node)| {
            let indent = "  ".repeat(node.depth());
            let icon = if node.is_dir() {
                if node.is_expanded() {
                    "\u{25bc} "
                } else {
                    "\u{25b6} "
                }
            } else {
                "  "
            };

            let name = node.name();
            let display = format!("{}{}{}", indent, icon, name);

            let style = if idx == selected_index {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if node.is_dir() {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(display).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" Files ")
                .title_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .style(Style::default().bg(Color::Rgb(30, 34, 42)));

    frame.render_widget(list, tree_area);

    // Render prompt line if there's a pending action
    if has_prompt && area.height > 2 {
        let prompt_area = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
        let prompt_line = match action {
            FileTreeAction::None => unreachable!(),
            FileTreeAction::Add { input, .. } => Line::from(vec![
                Span::styled("new: ", Style::default().fg(Color::Yellow)),
                Span::raw(input),
            ]),
            FileTreeAction::Rename { input, .. } => Line::from(vec![
                Span::styled("rename: ", Style::default().fg(Color::Yellow)),
                Span::raw(input),
            ]),
            FileTreeAction::DeleteConfirm { name, .. } => Line::from(vec![
                Span::styled("delete ", Style::default().fg(Color::Red)),
                Span::styled(name, Style::default().fg(Color::White)),
                Span::styled("? (y/N)", Style::default().fg(Color::Red)),
            ]),
        };
        let prompt = Paragraph::new(prompt_line).style(Style::default().bg(Color::Rgb(30, 34, 42)));
        frame.render_widget(prompt, prompt_area);
    }
}
