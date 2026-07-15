use super::Editor;

impl Editor {
    /// Selected message index in current conversation.
    pub fn ai_chat_history_selected_index(&self) -> Option<usize> {
        if self.ai_chat_history_selected_queued_id().is_some() {
            return None;
        }
        let conv = self.conversation()?;
        let node_ids = conv.node_ids_for_active_branch();
        if node_ids.is_empty() {
            return None;
        }
        let selected = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.history.selected_node_id);
        if let Some(sel) = selected {
            if let Some(idx) = node_ids.iter().position(|id| *id == sel) {
                return Some(idx);
            }
        }
        Some(node_ids.len() - 1)
    }

    pub fn ai_chat_history_selected_queued_id(&self) -> Option<u64> {
        let chat = self.ai_state.chat.as_ref()?;
        let id = chat.history.selected_queued_id?;
        chat.queued_inputs
            .iter()
            .any(|item| item.id == id)
            .then_some(id)
    }

    pub fn ai_chat_history_selected_queued_index(&self) -> Option<usize> {
        let id = self.ai_chat_history_selected_queued_id()?;
        self.ai_state
            .chat
            .as_ref()?
            .queued_inputs
            .iter()
            .position(|item| item.id == id)
    }

    /// Whether history selection currently points at latest message.
    pub fn ai_chat_history_is_latest_selected(&self) -> bool {
        let queued_len = self
            .ai_state
            .chat
            .as_ref()
            .map_or(0, |chat| chat.queued_inputs.len());
        if queued_len > 0 {
            return self.ai_chat_history_selected_queued_index() == Some(queued_len - 1);
        }
        let Some(idx) = self.ai_chat_history_selected_index() else {
            return true;
        };
        let len = self.ai_chat_messages().len();
        idx + 1 >= len
    }

    /// Effective row scroll offset for rendering given row count and viewport size.
    pub fn ai_chat_effective_message_scroll(
        &self,
        total_rows: usize,
        visible_rows: usize,
    ) -> usize {
        let Some(chat) = self.ai_state.chat.as_ref() else {
            return 0;
        };
        let viewport = &chat.viewport;
        if viewport.follow_latest || viewport.row_scroll_from_bottom == 0 {
            return 0;
        }
        let base = viewport.pinned_base_total_rows.unwrap_or(total_rows);
        let growth = total_rows.saturating_sub(base);
        let max_scroll = total_rows.saturating_sub(visible_rows);
        viewport
            .row_scroll_from_bottom
            .saturating_add(growth)
            .min(max_scroll)
    }

    /// Scroll chat history viewport toward older rows.
    pub fn ai_chat_scroll_viewport_up(&mut self, rows: usize) {
        if rows == 0 {
            return;
        }
        let baseline_rows = self.render_cache.ai_chat_last_total_rows;
        let baseline = if baseline_rows == 0 {
            None
        } else {
            Some(baseline_rows)
        };
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if chat.viewport.follow_latest {
                chat.viewport.follow_latest = false;
                chat.viewport.pinned_base_total_rows = baseline;
            } else if chat.viewport.pinned_base_total_rows.is_none() {
                chat.viewport.pinned_base_total_rows = baseline;
            }
            chat.viewport.row_scroll_from_bottom =
                chat.viewport.row_scroll_from_bottom.saturating_add(rows);
        }
    }

    /// Scroll chat history viewport toward latest rows.
    ///
    /// Returns true when the viewport reached bottom/latest.
    pub fn ai_chat_scroll_viewport_down(&mut self, rows: usize) -> bool {
        if rows == 0 {
            return self
                .ai_state
                .chat
                .as_ref()
                .is_some_and(|c| c.viewport.row_scroll_from_bottom == 0);
        }
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.viewport.row_scroll_from_bottom =
                chat.viewport.row_scroll_from_bottom.saturating_sub(rows);
            if chat.viewport.row_scroll_from_bottom == 0 {
                chat.viewport.follow_latest = true;
                chat.viewport.pinned_base_total_rows = None;
                return true;
            }
        }
        false
    }

    /// Move message-history selection toward older messages.
    pub fn ai_chat_history_cursor_move_older(&mut self, messages: usize) {
        if messages == 0 {
            return;
        }
        let node_ids = self
            .conversation()
            .map(|conv| conv.node_ids_for_active_branch().to_vec())
            .unwrap_or_default();
        let queued_ids = self
            .ai_state
            .chat
            .as_ref()
            .map(|chat| {
                chat.queued_inputs
                    .iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let total = node_ids.len() + queued_ids.len();
        if total == 0 {
            return;
        }
        let current = self
            .ai_chat_history_selected_queued_index()
            .map(|index| node_ids.len() + index)
            .or_else(|| self.ai_chat_history_selected_index())
            .unwrap_or(total - 1);
        let target = current.saturating_sub(messages);
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if target < node_ids.len() {
                chat.history.selected_node_id = node_ids.get(target).copied();
                chat.history.selected_queued_id = None;
            } else {
                chat.history.selected_node_id = None;
                chat.history.selected_queued_id = queued_ids.get(target - node_ids.len()).copied();
            }
        }
        self.ai_chat_history_ensure_cursor_visible();
    }

    /// Move message-history selection toward latest messages.
    ///
    /// Returns true when selection reached the latest message.
    pub fn ai_chat_history_cursor_move_newer(&mut self, messages: usize) -> bool {
        if messages == 0 {
            return self.ai_chat_history_is_latest_selected();
        }
        let node_ids = self
            .conversation()
            .map(|conv| conv.node_ids_for_active_branch().to_vec())
            .unwrap_or_default();
        let queued_ids = self
            .ai_state
            .chat
            .as_ref()
            .map(|chat| {
                chat.queued_inputs
                    .iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let total = node_ids.len() + queued_ids.len();
        if total == 0 {
            return true;
        }
        let current = self
            .ai_chat_history_selected_queued_index()
            .map(|index| node_ids.len() + index)
            .or_else(|| self.ai_chat_history_selected_index())
            .unwrap_or(total - 1);
        let target = current.saturating_add(messages).min(total - 1);
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if target < node_ids.len() {
                chat.history.selected_node_id = node_ids.get(target).copied();
                chat.history.selected_queued_id = None;
            } else {
                chat.history.selected_node_id = None;
                chat.history.selected_queued_id = queued_ids.get(target - node_ids.len()).copied();
            }
        }
        self.ai_chat_history_ensure_cursor_visible();
        self.ai_chat_history_is_latest_selected()
    }

    fn ai_chat_history_ensure_cursor_visible(&mut self) {
        let span = self
            .ai_chat_history_selected_queued_index()
            .and_then(|index| self.render_cache.ai_chat_last_queued_row_spans.get(index))
            .or_else(|| {
                self.ai_chat_history_selected_index()
                    .and_then(|index| self.render_cache.ai_chat_last_message_row_spans.get(index))
            })
            .copied();
        let Some((msg_start, msg_end)) = span else {
            return;
        };
        let vis_start = self.render_cache.ai_chat_last_visible_start_row;
        let vis_end = self.render_cache.ai_chat_last_visible_end_row;
        if vis_end <= vis_start {
            return;
        }

        if msg_end <= vis_start {
            let delta = vis_start.saturating_sub(msg_start).max(1);
            self.ai_chat_scroll_viewport_up(delta);
        } else if msg_start >= vis_end {
            let delta = msg_end.saturating_sub(vis_end).max(1);
            self.ai_chat_scroll_viewport_down(delta);
        }
    }

    /// Ensure history selection references the latest message.
    pub fn ai_chat_reset_history_cursor(&mut self) {
        let latest_queued = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.queued_inputs.back().map(|item| item.id));
        let latest = self
            .conversation()
            .and_then(|c| c.node_ids_for_active_branch().last().copied());
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.history.selected_queued_id = latest_queued;
            chat.history.selected_node_id = if latest_queued.is_none() {
                latest
            } else {
                None
            };
        }
    }
}
