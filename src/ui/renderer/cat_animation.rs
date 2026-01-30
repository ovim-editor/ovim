//! Dashboard cat animation — an idle easter egg.
//!
//! After ~45 seconds of idle, a tiny kawaii cat hops onto screen,
//! crouches, jumps on the logo, idles (sit, slow-blink, lick, pace),
//! and hops off. Terminal resize while on the logo triggers a
//! surprise → fall → splat comedy sequence.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    Frame,
};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Palette — warm, kawaii, Studio Ghibli-ish tones
// ---------------------------------------------------------------------------

/// Soft peach-orange tabby body.
const BODY: Color = Color::Rgb(210, 160, 110);
/// Bright teal-green eyes.
const EYE: Color = Color::Rgb(100, 215, 170);
/// Soft pink nose.
const NOSE: Color = Color::Rgb(240, 150, 170);
/// Wide startled eyes.
const SURPRISE_EYE: Color = Color::Rgb(140, 200, 240);
/// Knocked-out / hurt.
const HURT: Color = Color::Rgb(180, 140, 170);
/// Dizzy spiral eyes.
const DAZED_EYE: Color = Color::Rgb(210, 190, 110);
/// Golden stars / sparkles.
const SPARKLE: Color = Color::Rgb(255, 230, 140);

/// Map a sprite character to its palette color.
fn color_for(ch: char) -> Color {
    match ch {
        '◕' | '–' => EYE,
        'ᴥ' => NOSE,
        '⊙' => SURPRISE_EYE,
        '×' => HURT,
        '@' => DAZED_EYE,
        '✦' => SPARKLE,
        _ => BODY,
    }
}

/// Split a sprite line into colored [`Span`]s, grouping consecutive
/// characters of the same color for efficiency.
fn colored_spans(text: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur_color = BODY;
    let mut buf = String::new();

    for ch in text.chars() {
        // Spaces inherit the running color (invisible anyway).
        let c = if ch == ' ' { cur_color } else { color_for(ch) };
        if c == cur_color {
            buf.push(ch);
        } else {
            if !buf.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut buf),
                    Style::default().fg(cur_color),
                ));
            }
            cur_color = c;
            buf.push(ch);
        }
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, Style::default().fg(cur_color)));
    }
    spans
}

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

/// Milliseconds per animation frame.
const FRAME_MS: u128 = 130;

/// Seconds of idle before the cat appears. Long enough to be a genuine
/// easter egg — you have to leave the dashboard sitting there.
const IDLE_DELAY_SECS: u64 = 45;

// ---------------------------------------------------------------------------
// Sprites — unicode-enhanced kawaii cat
//
//  ╱╲_╱╲     Ears: box-drawing diagonals
// ( ◕ᴥ◕ )    Eyes: ◕ (teal), Nose: ᴥ (pink)
//  ╱│ │╲     Legs: box-drawing
//
// Walking uses a bouncy hop (y-offset alternates) with leg direction
// flipping between frames for a trotting effect.
// ---------------------------------------------------------------------------

// Ground walking — 3 rows, bouncy hop between frames.
const WALK1: &[&str] = &[" ╱╲_╱╲ ", "( ◕ᴥ◕ )", "  ╱ ╱  "];
const WALK2: &[&str] = &[" ╱╲_╱╲ ", "( ◕ᴥ◕ )", "  ╲ ╲  "];

// Crouching (preparing to jump) — sinks 1 row via blank top.
const CROUCH: &[&str] = &["        ", " ╱╲_╱╲ ", "( ◕ᴥ◕ )", " ╱> <╲ "];
// Butt-wiggle — tail swishes, eyes squint with focus.
const WIGGLE: &[&str] = &["        ", " ╱╲_╱╲ ", "( >ᴥ< )", "~╱> <╲ "];

// Mid-jump — legs tucked under.
const JUMP_UP: &[&str] = &[" ╱╲_╱╲ ", "( ◕ᴥ◕ )", " ╱> <╲ ", "        "];

// On-logo: relaxed sit.
const SIT: &[&str] = &[" ╱╲_╱╲ ", "( –ᴥ– )", " ╱│ │╲ "];
// On-logo: alert (eyes open).
const SIT_ALERT: &[&str] = &[" ╱╲_╱╲ ", "( ◕ᴥ◕ )", " ╱│ │╲ "];
// On-logo: licking paw.
const LICK: &[&str] = &[" ╱╲_╱╲  ", "( –ᴥ– )>", " ╱│ │╲  "];
// On-logo: mini walk.
const LOGO_WALK1: &[&str] = &[" ╱╲_╱╲ ", "( –ᴥ– )", "  ╱ ╱  "];
const LOGO_WALK2: &[&str] = &[" ╱╲_╱╲ ", "( –ᴥ– )", "  ╲ ╲  "];

// Startled (resize).
const SURPRISED: &[&str] = &[" ╱╲_╱╲ ", "( ⊙ᴥ⊙ )", " ╱│ │╲ "];
// Falling — arms flailing.
const FALL1: &[&str] = &["  ╱╲_╱╲  ", "╲( ⊙ᴥ⊙ )╱", "   │ │   "];
const FALL2: &[&str] = &["  ╱╲_╱╲  ", " ( ×ᴥ× ) ", " ╱│ │ │╲ "];
// Splat — landed hard.
const SPLAT: &[&str] = &["          ", "  ╱╲_╱╲   ", " ( ×ᴥ× )_ ", " _╱  ╲_╱  "];
// Stars circling head.
const DAZED_SPRITE: &[&str] = &["  ✦  ✦   ", "  ╱╲_╱╲  ", " ( @ᴥ@ ) ", "  ╱│ │╲  "];

/// Standard sprite width for centering calculations.
const SPRITE_W: usize = 8;

// ---------------------------------------------------------------------------
// Animation state machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Idle delay — nothing visible, waiting to become an easter egg.
    Waiting,
    /// Hopping rightward onto the screen.
    WalkOn,
    /// Crouching & wiggling under the logo.
    Crouch,
    /// Jumping upward to the logo.
    JumpUp,
    /// Idle on top of the logo (sit, blink, lick, pace).
    IdleOnLogo,
    /// Startled by resize.
    Surprised,
    /// Falling off the logo.
    Fall,
    /// Landed hard (splat).
    Landed,
    /// Seeing stars after landing.
    DazedRecovery,
    /// Graceful jump back down (normal exit).
    JumpDown,
    /// Hopping rightward off the screen.
    WalkOff,
    /// Animation finished.
    Done,
}

/// Persistent animation state. Create once, call [`tick()`] each frame.
pub struct CatAnimation {
    phase: Phase,
    /// Horizontal position (column) of sprite's left edge.
    x: i32,
    /// Frame counter within current phase.
    frame: usize,
    /// Total idle sub-frames on logo.
    idle_index: usize,
    /// Timestamp of last frame advance.
    last_advance: Instant,
    /// Timestamp of creation (for idle delay).
    created_at: Instant,
    /// The logo's vertical position (row).
    logo_y: u16,
    /// The logo's horizontal center.
    logo_center_x: u16,
    /// Screen width.
    screen_w: u16,
    /// Screen height.
    screen_h: u16,
}

impl CatAnimation {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            phase: Phase::Waiting,
            x: -10,
            frame: 0,
            idle_index: 0,
            last_advance: now,
            created_at: now,
            logo_y: 0,
            logo_center_x: 0,
            screen_w: 80,
            screen_h: 24,
        }
    }

    /// Returns true if the animation is still running (including waiting).
    pub fn is_active(&self) -> bool {
        self.phase != Phase::Done
    }

    /// Advance the animation clock. Returns true if a visual change
    /// occurred (caller should mark dirty).
    pub fn tick(&mut self) -> bool {
        if self.phase == Phase::Done {
            return false;
        }

        // Easter-egg delay: sit invisible until the terminal has been
        // idle long enough to surprise someone.
        if self.phase == Phase::Waiting {
            if self.created_at.elapsed().as_secs() >= IDLE_DELAY_SECS {
                self.phase = Phase::WalkOn;
                self.last_advance = Instant::now();
                return true;
            }
            return false;
        }

        let elapsed = self.last_advance.elapsed().as_millis();
        if elapsed < FRAME_MS {
            return false;
        }
        self.last_advance = Instant::now();
        self.advance();
        true
    }

    /// Startle the cat — called on terminal resize while it's on the logo.
    pub fn startle(&mut self) {
        if self.phase == Phase::IdleOnLogo {
            self.phase = Phase::Surprised;
            self.frame = 0;
        }
    }

    /// Update layout info from the dashboard renderer.
    pub fn set_layout(
        &mut self,
        logo_y: u16,
        logo_center_x: u16,
        screen_w: u16,
        screen_h: u16,
    ) {
        self.logo_y = logo_y;
        self.logo_center_x = logo_center_x;
        self.screen_w = screen_w;
        self.screen_h = screen_h;
    }

    // -- Phase transitions --------------------------------------------------

    fn advance(&mut self) {
        match self.phase {
            Phase::Waiting => {}

            Phase::WalkOn => {
                self.frame += 1;
                self.x += 2;
                let target_x = self.logo_center_x as i32 - (SPRITE_W as i32 / 2);
                if self.x >= target_x {
                    self.x = target_x;
                    self.phase = Phase::Crouch;
                    self.frame = 0;
                }
            }

            Phase::Crouch => {
                self.frame += 1;
                if self.frame >= 6 {
                    self.phase = Phase::JumpUp;
                    self.frame = 0;
                }
            }

            Phase::JumpUp => {
                self.frame += 1;
                if self.frame >= 3 {
                    self.phase = Phase::IdleOnLogo;
                    self.frame = 0;
                    self.idle_index = 0;
                }
            }

            Phase::IdleOnLogo => {
                self.frame += 1;
                self.idle_index += 1;
                // ~6 seconds of idle behaviors on the logo.
                if self.idle_index >= 48 {
                    self.phase = Phase::JumpDown;
                    self.frame = 0;
                }
            }

            Phase::Surprised => {
                self.frame += 1;
                if self.frame >= 5 {
                    self.phase = Phase::Fall;
                    self.frame = 0;
                }
            }

            Phase::Fall => {
                self.frame += 1;
                if self.frame >= 4 {
                    self.phase = Phase::Landed;
                    self.frame = 0;
                }
            }

            Phase::Landed => {
                self.frame += 1;
                if self.frame >= 6 {
                    self.phase = Phase::DazedRecovery;
                    self.frame = 0;
                }
            }

            Phase::DazedRecovery => {
                self.frame += 1;
                if self.frame >= 8 {
                    self.phase = Phase::WalkOff;
                    self.frame = 0;
                }
            }

            Phase::JumpDown => {
                self.frame += 1;
                if self.frame >= 3 {
                    self.phase = Phase::WalkOff;
                    self.frame = 0;
                }
            }

            Phase::WalkOff => {
                self.frame += 1;
                self.x += 2;
                if self.x > self.screen_w as i32 + 5 {
                    self.phase = Phase::Done;
                }
            }

            Phase::Done => {}
        }
    }

    // -- Rendering -----------------------------------------------------------

    /// Render the cat onto the frame.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if self.phase == Phase::Done || self.phase == Phase::Waiting {
            return;
        }

        let (sprite, y) = self.current_sprite_and_y(area);

        for (row_idx, line) in sprite.iter().enumerate() {
            let draw_y = y as i32 + row_idx as i32;
            if draw_y < area.y as i32 || draw_y >= (area.y + area.height) as i32 {
                continue;
            }

            // Horizontal clipping.
            let line_chars: Vec<char> = line.chars().collect();
            let mut visible = String::new();
            let mut col = self.x;

            for &ch in &line_chars {
                if col >= area.x as i32 && col < (area.x + area.width) as i32 {
                    visible.push(ch);
                } else if col >= (area.x + area.width) as i32 {
                    break;
                } else {
                    col += 1;
                    continue;
                }
                col += 1;
            }

            if visible.is_empty() {
                continue;
            }

            let render_x = self.x.max(area.x as i32) as u16;
            let render_y = draw_y as u16;

            let cat_line = Line::from(colored_spans(&visible));
            let rect = Rect {
                x: render_x,
                y: render_y,
                width: area.width.saturating_sub(render_x - area.x),
                height: 1,
            };

            frame.render_widget(
                ratatui::widgets::Paragraph::new(vec![cat_line]),
                rect,
            );
        }
    }

    /// Returns (sprite_lines, top_y) for the current animation state.
    fn current_sprite_and_y(&self, area: Rect) -> (&'static [&'static str], u16) {
        let ground_y = self.ground_y(area);
        let logo_top_y = self.logo_y;

        match self.phase {
            Phase::WalkOn | Phase::WalkOff => {
                let sprite = if self.frame % 2 == 0 { WALK1 } else { WALK2 };
                let base_y = ground_y.saturating_sub(sprite.len() as u16);
                // Bouncy hop: odd frames rise 1 row.
                let y = if self.frame % 2 == 1 {
                    base_y.saturating_sub(1)
                } else {
                    base_y
                };
                (sprite, y)
            }

            Phase::Crouch => {
                let sprite = if self.frame < 3 { CROUCH } else { WIGGLE };
                let y = ground_y.saturating_sub(sprite.len() as u16);
                (sprite, y)
            }

            Phase::JumpUp => {
                let sprite = JUMP_UP;
                let from = ground_y.saturating_sub(sprite.len() as u16) as f32;
                let to = logo_top_y.saturating_sub(sprite.len() as u16) as f32;
                let t = self.frame as f32 / 3.0;
                let y = (from + (to - from) * t).max(0.0) as u16;
                (sprite, y)
            }

            Phase::IdleOnLogo => {
                // Natural cat idle: sit → slow-blink → alert look → groom → pace → settle.
                let cycle = self.idle_index % 24;
                let sprite = match cycle {
                    0..=5 => SIT,
                    6 => SIT_ALERT,       // slow blink: eyes open
                    7..=8 => SIT,         // slow blink: eyes close
                    9..=12 => SIT_ALERT,  // alert, looking around
                    13..=16 => LICK,      // grooming
                    17..=18 => LOGO_WALK1, // little pace
                    19..=20 => LOGO_WALK2,
                    _ => SIT,             // settle back down
                };
                let y = logo_top_y.saturating_sub(sprite.len() as u16);
                (sprite, y)
            }

            Phase::Surprised => {
                let y = logo_top_y.saturating_sub(SURPRISED.len() as u16);
                (SURPRISED, y)
            }

            Phase::Fall => {
                let sprite = if self.frame % 2 == 0 { FALL1 } else { FALL2 };
                let from = logo_top_y.saturating_sub(sprite.len() as u16) as f32;
                let to = ground_y.saturating_sub(sprite.len() as u16) as f32;
                let t = self.frame as f32 / 4.0;
                let y = (from + (to - from) * t).max(0.0) as u16;
                (sprite, y)
            }

            Phase::Landed => {
                let y = ground_y.saturating_sub(SPLAT.len() as u16);
                (SPLAT, y)
            }

            Phase::DazedRecovery => {
                let sprite = if self.frame < 4 {
                    DAZED_SPRITE
                } else {
                    SIT_ALERT
                };
                let y = ground_y.saturating_sub(sprite.len() as u16);
                (sprite, y)
            }

            Phase::JumpDown => {
                let sprite = JUMP_UP;
                let from =
                    logo_top_y.saturating_sub(sprite.len() as u16) as f32;
                let to =
                    ground_y.saturating_sub(sprite.len() as u16) as f32;
                let t = self.frame as f32 / 3.0;
                let y = (from + (to - from) * t).max(0.0) as u16;
                (sprite, y)
            }

            Phase::Done | Phase::Waiting => (SIT, 0),
        }
    }

    /// Ground Y — cat's feet sit ~2 rows above the bottom.
    fn ground_y(&self, area: Rect) -> u16 {
        (area.y + area.height).saturating_sub(2)
    }
}
