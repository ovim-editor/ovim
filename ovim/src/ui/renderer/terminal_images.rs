use crate::editor::Editor;
use image::ImageReader;
use ratatui::{layout::Rect, Frame};
#[cfg(not(test))]
use ratatui_image::picker::ProtocolType;
use ratatui_image::{picker::Picker, protocol::Protocol, Image, Resize};
use std::collections::HashMap;
#[cfg(not(test))]
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct CacheKey {
    path: PathBuf,
    width: u16,
    height: u16,
}

pub struct TerminalImageRenderer {
    picker: Option<Picker>,
    protocols: HashMap<CacheKey, Protocol>,
    rendered_last_frame: bool,
}

impl TerminalImageRenderer {
    pub fn detect() -> Self {
        #[cfg(test)]
        return Self {
            picker: None,
            protocols: HashMap::new(),
            rendered_last_frame: false,
        };

        #[cfg(not(test))]
        {
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                return Self {
                    picker: None,
                    protocols: HashMap::new(),
                    rendered_last_frame: false,
                };
            }
            let picker = Picker::from_query_stdio()
                .ok()
                .filter(|picker| picker.protocol_type() != ProtocolType::Halfblocks);
            Self {
                picker,
                protocols: HashMap::new(),
                rendered_last_frame: false,
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.picker.is_some()
    }

    pub fn rendered_last_frame(&self) -> bool {
        self.rendered_last_frame
    }

    pub fn render(&mut self, frame: &mut Frame, editor: &Editor) {
        self.rendered_last_frame = false;
        if self.picker.is_none() {
            return;
        }

        let thumbnails = editor.render_cache.ai_chat_image_thumbnails.clone();
        for (area, path) in thumbnails {
            self.rendered_last_frame |= self.render_path(frame, &path, core_rect(area));
        }

        if let Some(path) = editor.ai_chat_image_modal_path() {
            let full = frame.area();
            if full.width >= 20 && full.height >= 10 {
                let outer_width = full.width * 4 / 5;
                let outer_height = full.height * 4 / 5;
                let area = Rect::new(
                    full.x + full.width.saturating_sub(outer_width) / 2 + 1,
                    full.y + full.height.saturating_sub(outer_height) / 2 + 1,
                    outer_width.saturating_sub(2),
                    outer_height.saturating_sub(2),
                );
                self.rendered_last_frame |= self.render_path(frame, path, area);
            }
        }

        // Keep long-running chats from retaining a decoded protocol for every
        // image/size combination ever shown.
        if self.protocols.len() > 64 {
            self.protocols.clear();
        }
    }

    fn render_path(&mut self, frame: &mut Frame, path: &Path, area: Rect) -> bool {
        if area.width == 0 || area.height == 0 {
            return false;
        }
        let key = CacheKey {
            path: path.to_path_buf(),
            width: area.width,
            height: area.height,
        };
        if !self.protocols.contains_key(&key) {
            let Some(picker) = self.picker.as_ref() else {
                return false;
            };
            let Ok(reader) = ImageReader::open(path) else {
                return false;
            };
            let Ok(image) = reader.decode() else {
                return false;
            };
            let Ok(protocol) = picker.new_protocol(
                image,
                Rect::new(0, 0, area.width, area.height),
                Resize::Fit(None),
            ) else {
                return false;
            };
            self.protocols.insert(key.clone(), protocol);
        }
        if let Some(protocol) = self.protocols.get(&key) {
            frame.render_widget(Image::new(protocol), area);
            true
        } else {
            false
        }
    }
}

fn core_rect(area: ovim_core::Rect) -> Rect {
    Rect::new(area.x, area.y, area.width, area.height)
}
