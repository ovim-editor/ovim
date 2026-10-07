use super::{chat_cursor_info, styled_word_wrap_line, LineRenderCache};
use crate::ui::renderer::ai_chat_bubble::{
    render_card_header_line, render_card_styled_line, render_card_text_line, render_chat_bubble,
};
use crate::ui::renderer::ai_chat_event_rows::{
    is_hidden_tool_only_assistant, render_queued_input_row, render_tool_event_details,
    render_tool_event_row,
};
use crate::ui::renderer::ai_chat_history::highlight_chat_selection;
use crate::ui::renderer::ai_chat_style::{
    MessageRowStyle, ACCENT_ASSISTANT_EDIT, BG_ASSISTANT_EDIT_ROW,
};
use crate::ui::renderer::ai_chat_text::{text_display_width, truncate_with_ellipsis, word_wrap};
use ovim_core::ai::chat_types::{ChatMessage, ChatRole, ImageAttachment, ToolCallInfo};
use ovim_core::editor::ai_chat_input::{chat_input_cursor_row_col, wrap_chat_input_rows};
use ovim_core::editor::{Editor, QueuedChatInputKind};
use ratatui::{
    backend::TestBackend,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    Terminal,
};

fn append_user_image_message(editor: &mut Editor, path: &str, content: &str) {
    let chat = editor.ai_state.chat.as_ref().unwrap();
    let key = (chat.origin_buffer_id, chat.opts.name.clone());
    editor
        .ai_state
        .conversations
        .get_mut(&key)
        .unwrap()
        .append_user_message_with_images(
            content.into(),
            vec![ImageAttachment {
                path: std::path::PathBuf::from(path),
                mime_type: "image/png".into(),
                data: vec![1, 2, 3],
            }],
        );
}

fn styled_row_text(row: &[Span<'static>]) -> String {
    row.iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

#[test]
fn conversation_wrap_keeps_words_intact_when_they_fit_a_row() {
    assert_eq!(word_wrap("alpha beta gamma", 10), ["alpha beta", "gamma"]);
    assert_eq!(word_wrap("abc defgh", 7), ["abc", "defgh"]);
}

#[test]
fn conversation_wrap_splits_only_words_wider_than_a_row() {
    assert_eq!(
        word_wrap("abc extraordinary tail", 5),
        ["abc", "extra", "ordin", "ary", "tail"]
    );
}

#[test]
fn styled_conversation_wrap_preserves_styles_across_word_boundaries() {
    let first_style = Style::default().fg(Color::Red);
    let second_style = Style::default().fg(Color::Blue);
    let line = Line::from(vec![
        Span::styled("alpha ", first_style),
        Span::styled("beta", second_style),
    ]);

    let rows = styled_word_wrap_line(&line, 7);
    assert_eq!(
        rows.iter()
            .map(|row| styled_row_text(row))
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert_eq!(rows[1][0].style, second_style);
}

#[test]
fn conversation_wrap_uses_terminal_display_width() {
    assert_eq!(word_wrap("ab 世界 cd", 5), ["ab", "世界", "cd"]);
}

#[test]
fn assistant_code_blocks_wrap_without_losing_content() {
    let theme = crate::syntax::Theme::default();
    for language in ["rust", "unknownlang12345", ""] {
        for code in [
            "let greeting = \"hei 👋 世界 verden\";",
            "alpha beta gamma delta epsilon",
            "abcdefghijklmnopqrstuvwxyz0123456789",
        ] {
            let message = ChatMessage {
                role: ChatRole::Assistant,
                content: format!("```{language}\n{code}\n```"),
                model: None,
                timestamp: std::time::Instant::now(),
                images: vec![],
                tool_calls: vec![],
                tool_call_id: None,
                provider_state: vec![],
            };
            for width in [12, 24, 80] {
                let bubble = render_chat_bubble(
                    &message, width, false, false, false, 0, None, &theme, false,
                );
                let rows = bubble
                    .lines
                    .iter()
                    .skip(1)
                    .map(|line| {
                        assert!(line.width() <= width);
                        styled_row_text(&line.spans)["▍ ".len()..]
                            .trim()
                            .to_string()
                    })
                    .collect::<Vec<_>>();
                let rendered = rows.join(" ");
                assert!(!rendered.contains("..."));
                assert!(!rendered.contains('…'));
                let without_whitespace = |text: &str| {
                    text.chars()
                        .filter(|character| !character.is_whitespace())
                        .collect::<String>()
                };
                assert_eq!(without_whitespace(&rendered), without_whitespace(code));
                if code.starts_with("alpha") && width == 12 {
                    assert_eq!(rows, ["alpha", "beta gamma", "delta", "epsilon"]);
                }
            }
        }
    }
}

fn claude_editor() -> Editor {
    let mut editor = Editor::default();
    let mut profile = editor.ai_state.config.profiles["local"].clone();
    profile.name = "claude_code".into();
    profile.provider = ovim_core::ai::AiProviderKind::ClaudeCode;
    profile.model = "default".into();
    editor
        .ai_state
        .config
        .profiles
        .insert(profile.name.clone(), profile);
    editor
        .open_ai_chat(ovim_core::ai::ChatOpts::default())
        .unwrap();
    assert!(editor.ai_select_chat_profile("claude_code"));
    editor
}

fn rendered_header(editor: &mut Editor, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
    terminal
        .draw(|frame| {
            super::render_chat_header(frame, editor, Rect::new(0, 0, width, 1));
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn header_warns_at_every_width_while_claude_bypasses_permissions() {
    let mut editor = claude_editor();
    for width in [30, 40, 80] {
        assert!(!rendered_header(&mut editor, width).contains("BYPASS"));
    }
    assert!(!editor.set_ai_chat_permission_mode("bypassPermissions"));
    assert!(!rendered_header(&mut editor, 40).contains("BYPASS"));
    assert!(editor.set_ai_chat_permission_mode("bypassPermissions"));
    for width in [30, 40, 80] {
        let header = rendered_header(&mut editor, width);
        assert!(header.contains("BYPASS"), "{width}: {header:?}");
    }
    assert!(rendered_header(&mut editor, 80).contains("BYPASS PERMISSIONS"));
    assert!(editor.set_ai_chat_permission_mode("plan"));
    assert!(!rendered_header(&mut editor, 40).contains("BYPASS"));
}

#[test]
fn header_shows_a_session_wide_shell_grant() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::ChatOpts::default())
        .unwrap();
    assert!(!rendered_header(&mut editor, 100).contains("SHELL ALLOWED"));
    editor.ai_state.chat.as_mut().unwrap().shell_allowed_session = true;
    let header = rendered_header(&mut editor, 100);
    assert!(header.contains("SHELL ALLOWED"), "{header:?}");
    // The grant badge must not displace the policy toggles it sits beside.
    assert!(header.contains("YOLO"), "{header:?}");
    assert!(header.contains("COMPREHENSION"), "{header:?}");
}

#[test]
fn permission_picker_never_offers_bypass_as_a_row() {
    let mut editor = claude_editor();
    editor.open_ai_chat_model_picker(ovim_core::editor::ChatModelPickerSection::Permission);
    let mut terminal = Terminal::new(TestBackend::new(60, 40)).unwrap();
    terminal
        .draw(|frame| super::render_model_picker(frame, &mut editor, None, Rect::new(0, 0, 60, 40)))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("Don't ask"));
    assert!(!rendered.contains("Bypass"));
    assert!(editor
        .render_cache
        .ai_chat_interactions
        .permission_picker_options
        .iter()
        .all(|(_, mode)| mode != "bypassPermissions"));
}

#[test]
fn permission_picker_keeps_selected_mode_and_mouse_target_visible_in_short_panel() {
    let mut editor = Editor::default();
    let mut profile = editor.ai_state.config.profiles["local"].clone();
    profile.name = "claude_code".into();
    profile.provider = ovim_core::ai::AiProviderKind::ClaudeCode;
    profile.model = "default".into();
    editor
        .ai_state
        .config
        .profiles
        .insert(profile.name.clone(), profile);
    editor
        .open_ai_chat(ovim_core::ai::ChatOpts::default())
        .unwrap();
    assert!(editor.ai_select_chat_profile("claude_code"));
    // Enabling bypass takes two identical commands; it is never a picker row.
    assert!(!editor.set_ai_chat_permission_mode("bypassPermissions"));
    assert!(editor.set_ai_chat_permission_mode("bypassPermissions"));
    editor.open_ai_chat_model_picker(ovim_core::editor::ChatModelPickerSection::Permission);
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|frame| super::render_model_picker(frame, &mut editor, None, Rect::new(0, 0, 40, 10)))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("● Bypass permissions"));
    let (area, _) = editor
        .render_cache
        .ai_chat_interactions
        .permission_picker_options
        .iter()
        .find(|(_, mode)| mode == "bypassPermissions")
        .unwrap();
    assert!(area.y > 0 && area.y < 9);
    assert_eq!(terminal.backend().buffer()[(area.x, area.y)].symbol(), "●");
}

#[test]
fn partial_slash_command_renders_completion_popup_and_hitboxes() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    let chat = editor.ai_state.chat.as_mut().unwrap();
    chat.input = "/".into();
    chat.input_cursor = 1;
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

    terminal
        .draw(|frame| {
            super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
        })
        .unwrap();

    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Commands"));
    assert!(rendered.contains("/clear"));
    assert!(rendered.contains("/model [profile|model]"));
    assert!(rendered.contains("/effort"));
    assert!(rendered.contains("/comprehension"));
    assert_eq!(
        editor
            .render_cache
            .ai_chat_interactions
            .slash_completions
            .len(),
        6
    );
}

#[test]
fn wrap_input_rows_preserves_trailing_space() {
    let input = "abc ";
    let rows = wrap_chat_input_rows(input, 20, 4);
    assert_eq!(&input[rows[0].start..rows[0].end], "abc ");
}

#[test]
fn cursor_stays_on_same_row_after_trailing_space() {
    let input = "abc ";
    let rows = wrap_chat_input_rows(input, 20, 4);
    let (row, col) = chat_input_cursor_row_col(input, input.len(), &rows, 4);
    assert_eq!(row, 0);
    assert_eq!(col, 4);
}

#[test]
fn cursor_moves_to_next_row_after_newline() {
    let input = "abc\n";
    let rows = wrap_chat_input_rows(input, 20, 4);
    let (row, col) = chat_input_cursor_row_col(input, input.len(), &rows, 4);
    assert_eq!(row, 1);
    assert_eq!(col, 0);
}

#[test]
fn composer_cursor_stays_inside_panel_after_height_cap() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    let input = (0..20)
        .map(|index| format!("word{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let chat = editor.ai_state.chat.as_mut().unwrap();
    chat.input = input;
    chat.input_cursor = chat.input.len();
    let panel = Rect {
        x: 40,
        y: 2,
        width: 42,
        height: 18,
    };

    let (x, y) = chat_cursor_info(&editor, panel).unwrap();

    assert!(x >= panel.x && x < panel.x + panel.width);
    assert!(y >= panel.y && y < panel.y + panel.height);
    assert_eq!(y, panel.y + panel.height - 1);
}

#[test]
fn supported_terminal_reserves_clickable_thumbnail_strip() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    editor.render_cache.terminal_image_support = true;
    editor.ai_state.chat.as_mut().unwrap().pending_images.push(
        ovim_core::ai::chat_types::ImageAttachment {
            path: std::path::PathBuf::from("/tmp/preview.png"),
            mime_type: "image/png".into(),
            data: vec![1, 2, 3],
        },
    );
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

    terminal
        .draw(|frame| {
            super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
        })
        .unwrap();

    assert_eq!(editor.render_cache.ai_chat_image_thumbnails.len(), 1);
    assert_eq!(
        editor.render_cache.ai_chat_image_thumbnails[0].1,
        std::path::PathBuf::from("/tmp/preview.png")
    );
}

#[test]
fn streaming_assistant_display_math_emits_an_image_placement() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    editor.render_cache.terminal_image_support = true;
    editor.ai_state.chat.as_mut().unwrap().streaming_content =
        Some("In three dimensions:\n\\[\nu(x,y,z,t)=\\begin{pmatrix}u_1\\\\u_2\\\\u_3\\end{pmatrix}\n\\]\nContinuing".into());
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);

    loop {
        terminal
            .draw(|frame| {
                super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
            })
            .unwrap();
        if editor
            .render_cache
            .ai_chat_image_thumbnails
            .iter()
            .any(|(_, path)| path.starts_with(std::env::temp_dir().join("ovim-display-math")))
        {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "streaming math image was never placed; total={}, visible={}..{}, rows={:?}, images={:?}",
                editor.render_cache.ai_chat_last_total_rows,
                editor.render_cache.ai_chat_last_visible_start_row,
                editor.render_cache.ai_chat_last_visible_end_row,
                editor.render_cache.ai_chat_rendered_text_rows,
                editor.render_cache.ai_chat_image_thumbnails,
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn assistant_tables_switch_between_grid_and_stacked_layouts() {
    let message = ChatMessage {
        role: ChatRole::Assistant,
        content: "| Area | Nula | Nushell |\n|---|---|---|\n| Primary role | Embedded data transformation | Interactive system shell |".into(),
        model: Some("model".into()),
        timestamp: std::time::Instant::now(),
        images: vec![],
        tool_calls: vec![],
        tool_call_id: None,
        provider_state: vec![],
    };
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

    let narrow = render_chat_bubble(&message, 42, false, false, false, 0, None, &theme, false);
    let narrow_text = narrow
        .lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(narrow_text
        .iter()
        .any(|line| line.contains("Area: Primary role")));
    assert!(narrow_text
        .iter()
        .all(|line| text_display_width(line) == 42));

    let wide = render_chat_bubble(&message, 100, false, false, false, 0, None, &theme, false);
    let wide_text = wide
        .lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(wide_text.iter().any(|line| line.contains('┌')));
    assert!(wide_text.iter().all(|line| text_display_width(line) == 100));
}

#[test]
fn chat_header_renders_clickable_yolo_state_at_top_right() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

    terminal
        .draw(|frame| {
            super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
        })
        .unwrap();
    let header = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .take(80)
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(header.contains("YOLO OFF"), "{header}");
    assert!(header.contains("COMPREHENSION OFF"), "{header}");
    let hitbox = editor
        .render_cache
        .ai_chat_interactions
        .yolo_toggle
        .unwrap();
    assert_eq!(hitbox.y, 0);
    assert_eq!(hitbox.x + hitbox.width, 80);
    let comprehension_hitbox = editor
        .render_cache
        .ai_chat_interactions
        .comprehension_toggle
        .unwrap();
    assert_eq!(comprehension_hitbox.y, 0);
    assert_eq!(
        comprehension_hitbox.x + comprehension_hitbox.width,
        hitbox.x
    );

    assert!(editor.set_ai_chat_yolo_mode(true));
    terminal
        .draw(|frame| {
            super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
        })
        .unwrap();
    let header = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .take(80)
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(header.contains("YOLO ON"), "{header}");
}

#[test]
fn claude_model_picker_scrolls_and_maps_visible_mouse_targets() {
    let mut editor = Editor::default();
    let mut profile = editor.ai_state.config.profiles["local"].clone();
    profile.name = "claude_code".into();
    profile.provider = ovim_core::ai::AiProviderKind::ClaudeCode;
    profile.model = "default".into();
    editor
        .ai_state
        .config
        .profiles
        .insert(profile.name.clone(), profile);
    editor
        .open_ai_chat(ovim_core::ai::ChatOpts::default())
        .unwrap();
    assert!(editor.ai_select_chat_model("claude_code", "claude-custom-version[1m]"));
    editor.open_ai_chat_model_picker(ovim_core::editor::ChatModelPickerSection::Model);
    let mut terminal = Terminal::new(TestBackend::new(70, 8)).unwrap();
    terminal
        .draw(|frame| {
            super::render_model_picker(
                frame,
                &mut editor,
                Some(Rect::new(0, 0, 50, 1)),
                Rect::new(0, 0, 70, 8),
            );
        })
        .unwrap();
    let interactions = &editor.render_cache.ai_chat_interactions;
    assert!(interactions
        .model_picker_options
        .iter()
        .any(|(area, option)| option.model == "claude-custom-version[1m]" && area.y < 7));
    let (area, _) = interactions
        .model_picker_options
        .iter()
        .find(|(_, option)| option.model == "claude-fable-5-1")
        .unwrap();
    let event = ovim_core::MouseEvent {
        kind: ovim_core::MouseEventKind::Down(ovim_core::MouseButton::Left),
        column: area.x,
        row: area.y,
    };
    ovim_core::editor::handle_mouse_event(&mut editor, event).unwrap();
    assert_eq!(editor.ai_chat_selected_model(), "claude-fable-5-1");
}

#[test]
fn chat_header_model_and_effort_picker_expands_downward() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    editor.open_ai_chat_model_picker(ovim_core::editor::ChatModelPickerSection::Effort);
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

    terminal
        .draw(|frame| {
            super::render_chat_panel(frame, &mut editor, Rect::new(0, 0, 100, 22), &theme)
        })
        .unwrap();

    let interactions = &editor.render_cache.ai_chat_interactions;
    let model = interactions.model_picker_trigger.expect("model trigger");
    let effort = interactions.effort_picker_trigger.expect("effort trigger");
    let comprehension = interactions
        .comprehension_toggle
        .expect("comprehension trigger");
    assert_eq!(model.y, 0);
    assert_eq!(model.x + model.width, effort.x);
    assert_eq!(effort.x + effort.width, comprehension.x);
    assert!(!interactions.model_picker_options.is_empty());
    assert!(!interactions.effort_picker_options.is_empty());
    assert!(interactions
        .model_picker_options
        .iter()
        .map(|(area, _)| area)
        .chain(
            interactions
                .effort_picker_options
                .iter()
                .map(|(area, _)| area)
        )
        .all(|area| area.y > model.y));

    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("MODEL PROFILES"), "{rendered}");
    assert!(rendered.contains("REASONING EFFORT"), "{rendered}");
}

#[test]
fn claude_profile_renders_in_terminal_without_ovim_policy_controls() {
    let mut editor = Editor::default();
    let mut profile = editor.ai_state.config.profiles["local"].clone();
    profile.provider = ovim_core::ai::AiProviderKind::ClaudeCode;
    profile.name = "claude_code".into();
    profile.model = "default".into();
    editor
        .ai_state
        .config
        .profiles
        .insert(profile.name.clone(), profile);
    assert!(editor.ai_set_profile("claude_code"));
    editor
        .open_ai_chat(ovim_core::ai::ChatOpts {
            profile: Some("claude_code".into()),
            ..Default::default()
        })
        .unwrap();
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());
    terminal
        .draw(|frame| {
            super::render_chat_panel(frame, &mut editor, Rect::new(0, 0, 100, 22), &theme);
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Claude Agent"), "{rendered}");
    assert!(!rendered.contains("YOLO"), "{rendered}");
    assert!(!rendered.contains("COMPREHENSION"), "{rendered}");
    assert!(editor
        .render_cache
        .ai_chat_interactions
        .yolo_toggle
        .is_none());
    assert!(editor
        .render_cache
        .ai_chat_interactions
        .comprehension_toggle
        .is_none());
}

#[test]
fn sent_image_thumbnail_is_positioned_inside_its_visible_message() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    editor.render_cache.terminal_image_support = true;
    append_user_image_message(&mut editor, "/tmp/sent-preview.png", "inspect this");

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());
    terminal
        .draw(|frame| {
            super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
        })
        .unwrap();

    assert_eq!(editor.render_cache.ai_chat_image_thumbnails.len(), 1);
    let (thumbnail, path) = &editor.render_cache.ai_chat_image_thumbnails[0];
    assert_eq!(path, &std::path::PathBuf::from("/tmp/sent-preview.png"));
    assert!(thumbnail.y < editor.render_cache.ai_chat_input_area.unwrap().y);
    assert!(editor.ai_chat_pending_images().is_empty());
}

#[test]
fn offscreen_message_image_does_not_enqueue_terminal_rendering() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    editor.render_cache.terminal_image_support = true;
    append_user_image_message(&mut editor, "/tmp/offscreen.png", "old image");
    let chat = editor.ai_state.chat.as_ref().unwrap();
    let key = (chat.origin_buffer_id, chat.opts.name.clone());
    let conversation = editor.ai_state.conversations.get_mut(&key).unwrap();
    for index in 0..16 {
        conversation.append_assistant_message(format!("later response {index}"), "model".into());
    }

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());
    terminal
        .draw(|frame| {
            super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
        })
        .unwrap();

    assert!(editor.render_cache.ai_chat_image_thumbnails.is_empty());
}

#[test]
fn card_rows_preserve_explicit_markdown_backgrounds() {
    let code_bg = Color::Rgb(1, 2, 3);
    let line = render_card_styled_line(
        24,
        "▍",
        ACCENT_ASSISTANT_EDIT,
        BG_ASSISTANT_EDIT_ROW,
        vec![Span::styled(
            "inline",
            Style::default().fg(Color::White).bg(code_bg),
        )],
    );

    let code_span = line
        .spans
        .iter()
        .find(|span| span.content == "inline")
        .expect("rendered inline-code span");
    assert_eq!(code_span.style.bg, Some(code_bg));
    assert_eq!(
        line.spans.last().unwrap().style.bg,
        Some(BG_ASSISTANT_EDIT_ROW)
    );
}

#[test]
fn completed_chat_bubbles_are_reused_across_frames() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    let chat = editor.ai_state.chat.as_ref().unwrap();
    let key = (chat.origin_buffer_id, chat.opts.name.clone());
    let conversation = editor.ai_state.conversations.get_mut(&key).unwrap();
    conversation.append_user_message("Explain this".into());
    conversation.append_assistant_message(
        "A **markdown** response with `code`.".into(),
        "model".into(),
    );

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());
    let mut cache = LineRenderCache::new();

    terminal
        .draw(|frame| {
            super::render_chat_panel_cached(
                frame,
                &mut editor,
                Rect::new(40, 0, 40, 22),
                &theme,
                &mut cache,
            )
        })
        .unwrap();
    assert!(cache.chat_misses >= 2);

    cache.reset_stats();
    terminal
        .draw(|frame| {
            super::render_chat_panel_cached(
                frame,
                &mut editor,
                Rect::new(40, 0, 40, 22),
                &theme,
                &mut cache,
            )
        })
        .unwrap();

    assert!(cache.chat_hits >= 2);
    assert_eq!(cache.chat_misses, 0);
}

#[test]
fn forked_message_renders_clickable_sibling_control() {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
        .unwrap();
    let (main_user, fork_user) = {
        let chat = editor.ai_state.chat.as_ref().unwrap();
        let key = (chat.origin_buffer_id, chat.opts.name.clone());
        let conversation = editor.ai_state.conversations.get_mut(&key).unwrap();
        conversation.append_user_message("first".into());
        let first_reply = conversation.append_assistant_message("reply".into(), "model".into());
        let main_user = conversation.append_user_message("main continuation".into());
        conversation.append_assistant_message("main reply".into(), "model".into());
        conversation.fork_from(first_reply);
        let fork_user = conversation.append_user_message("fork continuation".into());
        conversation.append_assistant_message("fork reply".into(), "model".into());
        (main_user, fork_user)
    };
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

    terminal
        .draw(|frame| {
            super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
        })
        .unwrap();

    let rendered = terminal.backend().buffer().content().to_vec();
    let rendered_text = rendered
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered_text.contains("[‹ 2/2 ›]"));
    assert_eq!(editor.render_cache.ai_chat_interactions.branches.len(), 2);
    assert_eq!(
        editor.render_cache.ai_chat_interactions.branches[0].1,
        main_user
    );
    assert_eq!(
        editor.render_cache.ai_chat_interactions.branches[1].1,
        main_user
    );
    assert_ne!(main_user, fork_user);
}

#[test]
fn hides_empty_assistant_messages_with_only_tool_calls() {
    let msg = ChatMessage {
        role: ChatRole::Assistant,
        content: "  ".to_string(),
        model: Some("model".to_string()),
        timestamp: std::time::Instant::now(),
        images: vec![],
        tool_calls: vec![ToolCallInfo {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({}),
        }],
        tool_call_id: None,
        provider_state: vec![],
    };
    assert!(is_hidden_tool_only_assistant(&msg));
}

#[test]
fn does_not_hide_non_empty_assistant_messages() {
    let msg = ChatMessage {
        role: ChatRole::Assistant,
        content: "done".to_string(),
        model: Some("model".to_string()),
        timestamp: std::time::Instant::now(),
        images: vec![],
        tool_calls: vec![ToolCallInfo {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({}),
        }],
        tool_call_id: None,
        provider_state: vec![],
    };
    assert!(!is_hidden_tool_only_assistant(&msg));
}

#[test]
fn queued_commands_are_labeled_distinctly() {
    let line = render_queued_input_row(40, QueuedChatInputKind::Command, "/clear", 0, false);
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(text.contains("/ command: /clear"));
}

#[test]
fn selected_queued_input_is_visually_emphasized() {
    let line = render_queued_input_row(40, QueuedChatInputKind::FollowUp, "next", 0, true);
    assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    assert!(!line.spans[0].style.add_modifier.contains(Modifier::DIM));
}

#[test]
fn expanded_tool_details_include_arguments_and_result() {
    let call = ToolCallInfo {
        id: "call_1".to_string(),
        name: "read_file".to_string(),
        arguments: serde_json::json!({"path": "src/main.rs"}),
    };
    let lines = render_tool_event_details(80, Some(&call), "Target: src/main.rs\nfn main() {}");
    let text = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("tool: read_file"));
    assert!(text.contains("src/main.rs"));
    assert!(text.contains("fn main() {}"));
}

#[test]
fn walkthrough_history_row_reserves_a_right_aligned_replay_action() {
    let line = render_tool_event_row(
        48,
        "walkthrough · 17 pages",
        ovim_core::ai::chat_types::ToolSummaryKind::Navigation,
        false,
        false,
        false,
        Some("[↻ replay]"),
    );
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert_eq!(text.chars().count(), 48);
    assert!(text.ends_with("[↻ replay]"));
    assert!(line
        .spans
        .last()
        .unwrap()
        .style
        .add_modifier
        .contains(Modifier::BOLD));
}

#[test]
fn wide_char_tool_row_pads_by_display_width_and_keeps_replay_visible() {
    // CJK characters occupy two display columns each; the label span must
    // be budgeted in columns so the right-aligned replay action still
    // lands inside the panel (matching its click hitbox).
    let line = render_tool_event_row(
        48,
        "代码漫游 · 十七个步骤的完整讲解流程",
        ovim_core::ai::chat_types::ToolSummaryKind::Navigation,
        false,
        false,
        false,
        Some("[↻ replay]"),
    );
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert_eq!(text_display_width(&text), 48);
    assert!(text.ends_with("[↻ replay]"));
}

#[test]
fn wide_char_queued_row_pads_by_display_width() {
    let line = render_queued_input_row(
        24,
        QueuedChatInputKind::FollowUp,
        "日本語のテキストがとても長い場合",
        0,
        false,
    );
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert_eq!(text_display_width(&text), 24);
}

#[test]
fn wide_char_tool_details_pad_by_display_width() {
    let lines = render_tool_event_details(20, None, "宽字符宽字符宽字符宽字符宽字符");
    for line in lines {
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text_display_width(&text), 20);
    }
}

#[test]
fn wide_char_message_row_pads_to_exact_panel_width() {
    // CJK text occupies two columns per character; padding must be
    // computed from display width or the row overflows the panel.
    for text in [
        "日本語テキスト",
        "短い",
        "宽字符宽字符宽字符宽字符宽字符宽字符",
    ] {
        let line = render_card_text_line(
            24,
            "\u{258d}",
            Color::White,
            Color::Reset,
            text,
            Style::default(),
        );
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text_display_width(&rendered), 24, "text: {text}");
    }
}

#[test]
fn wide_char_header_pads_to_exact_panel_width() {
    let row_style = MessageRowStyle {
        accent: Color::White,
        label_fg: Color::White,
        label_bg: Color::Reset,
        text_fg: Color::White,
        body_bg: Color::Reset,
    };
    for branch in [None, Some((0, 3))] {
        let line = render_card_header_line(
            24,
            "\u{258d}",
            row_style,
            "日本語のラベルがとても長い場合",
            branch,
        );
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(
            text_display_width(&rendered),
            24,
            "branch: {branch:?}, rendered: {rendered:?}"
        );
    }
}

#[test]
fn zwj_emoji_measures_as_single_grapheme_width() {
    // "👩‍🔬" is woman + ZWJ + microscope: one grapheme, 2 columns.
    // Summing per-char widths would report 4 and overpad the row.
    assert_eq!(text_display_width("👩\u{200d}🔬"), 2);
}

#[test]
fn truncation_never_splits_zwj_emoji_sequence() {
    let text = "👩\u{200d}🔬👩\u{200d}🔬";
    // Wide enough: untouched.
    assert_eq!(truncate_with_ellipsis(text, 4), text);
    // Budget of 2 columns after the ellipsis: keeps the first full
    // sequence, never a dangling "👩" or ZWJ.
    assert_eq!(truncate_with_ellipsis(text, 3), "👩\u{200d}🔬\u{2026}");
    // Budget too small for the sequence: drops it entirely.
    assert_eq!(truncate_with_ellipsis(text, 2), "\u{2026}");
    assert_eq!(truncate_with_ellipsis(text, 1), "\u{2026}");
}

#[test]
fn saved_review_history_rows_register_replay_hitboxes() {
    for name in ["explain_with_codebase", "show_custom_diff"] {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
            .unwrap();
        {
            let chat = editor.ai_state.chat.as_ref().unwrap();
            let key = (chat.origin_buffer_id, chat.opts.name.clone());
            let conversation = editor.ai_state.conversations.get_mut(&key).unwrap();
            conversation.append_assistant_message_with_tools(
                String::new(),
                "model".into(),
                vec![ToolCallInfo {
                    id: "walkthrough-call".into(),
                    name: name.into(),
                    arguments: serde_json::json!({"steps": []}),
                }],
            );
            conversation.append_tool_result(
                "walkthrough-call".into(),
                if name == "show_custom_diff" {
                    serde_json::json!({"ovim_custom_diff_id": "walkthrough-call"}).to_string()
                } else {
                    "User completed the code walkthrough (17 steps).".into()
                },
            );
        }
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

        terminal
            .draw(|frame| {
                super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
            })
            .unwrap();

        assert_eq!(
            editor.render_cache.ai_chat_interactions.tool_replays.len(),
            1
        );
        assert_eq!(
            editor.render_cache.ai_chat_interactions.tool_replays[0].1,
            "walkthrough-call"
        );
    }
}

#[test]
fn chat_text_selection_highlights_only_the_selected_columns() {
    let line = Line::from(Span::styled("abcdef", Style::default().fg(Color::Green)));
    let highlighted = highlight_chat_selection(&line, 2, 4);
    assert_eq!(
        highlighted
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>(),
        "abcdef"
    );
    assert!(highlighted
        .spans
        .iter()
        .any(|span| { span.content == "cd" && span.style.bg == Some(Color::Rgb(74, 96, 145)) }));
}
