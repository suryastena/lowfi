//! A modular UI component system for lowfi
//! This provides a flexible trait-based architecture for UI components

use std::sync::Arc;
use std::time::Duration;
use crossterm::style::Stylize as _;
use unicode_segmentation::UnicodeSegmentation as _;

/// Core trait for all UI components
pub trait UIComponent: Send + Sync {
    /// Render the component to a string
    fn render(&self, context: &RenderContext) -> String;
    
    /// Get the minimum width required for this component
    fn min_width(&self) -> usize {
        0
    }
    
    /// Get the preferred height for this component
    fn height(&self) -> usize {
        1
    }
    
    /// Whether this component should be visible
    fn is_visible(&self) -> bool {
        true
    }
    
    /// Handle an update event
    fn handle_event(&mut self, _event: ComponentEvent) -> EventResult {
        EventResult::Ignored
    }
}

/// Result of handling an event
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventResult {
    /// Event was consumed
    Consumed,
    /// Event was ignored and should bubble up
    Ignored,
    /// Request a redraw
    Redraw,
}

/// Events that components can handle
#[derive(Debug, Clone)]
pub enum ComponentEvent {
    /// Volume changed
    VolumeChanged(f32),
    /// Playback state changed
    PlaybackStateChanged(PlaybackState),
    /// Track changed
    TrackChanged(TrackInfo),
    /// Progress update
    ProgressUpdate(Duration, Option<Duration>),
    /// Bookmark state changed
    BookmarkChanged(bool),
    /// Custom event with optional data
    Custom(String),
}

/// Playback state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Loading,
}

/// Track information
#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub name: String,
    pub display_name: String,
    pub width: usize,
    pub duration: Option<Duration>,
}

/// Context provided to components for rendering
pub struct RenderContext {
    /// Available width for rendering
    pub width: usize,
    /// Current playback state
    pub playback_state: PlaybackState,
    /// Current track info, if any
    pub track_info: Option<Arc<TrackInfo>>,
    /// Current volume (0.0 - 1.0)
    pub volume: f32,
    /// Current playback position
    pub position: Duration,
    /// Whether track is bookmarked
    pub is_bookmarked: bool,
    /// Whether borders should be shown
    pub borderless: bool,
    /// Any custom data
    pub custom_data: std::collections::HashMap<String, String>,
}

/// A component that can contain other components
pub trait Container: UIComponent {
    /// Add a child component
    fn add_child(&mut self, component: Box<dyn UIComponent>);
    
    /// Remove a child component by index
    fn remove_child(&mut self, index: usize) -> Option<Box<dyn UIComponent>>;
    
    /// Get the number of children
    fn child_count(&self) -> usize;
    
    /// Get a mutable reference to a child
    fn get_child_mut(&mut self, index: usize) -> Option<&mut dyn UIComponent>;
}

// ============= Basic Component Implementations =============

/// A simple text label component
pub struct Label {
    text: String,
    style: TextStyle,
}

#[derive(Clone, Debug)]
pub enum TextStyle {
    Normal,
    Bold,
    Italic,
    Underline,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: TextStyle::Normal,
        }
    }
    
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }
}

impl UIComponent for Label {
    fn render(&self, _context: &RenderContext) -> String {
        match self.style {
            TextStyle::Normal => self.text.clone(),
            TextStyle::Bold => format!("{}", self.text.bold()),
            TextStyle::Italic => format!("{}", self.text.italic()),
            TextStyle::Underline => format!("{}", self.text.underlined()),
        }
    }
    
    fn min_width(&self) -> usize {
        self.text.graphemes(true).count()
    }
}

/// Progress bar component
pub struct ProgressBar {
    fill_char: char,
    empty_char: char,
    show_time: bool,
}

impl Default for ProgressBar {
    fn default() -> Self {
        Self {
            fill_char: '/',
            empty_char: ' ',
            show_time: true,
        }
    }
}

impl ProgressBar {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn with_chars(mut self, fill: char, empty: char) -> Self {
        self.fill_char = fill;
        self.empty_char = empty;
        self
    }
    
    pub fn with_time_display(mut self, show: bool) -> Self {
        self.show_time = show;
        self
    }
    
    fn format_duration(duration: &Duration) -> String {
        let seconds = duration.as_secs() % 60;
        let minutes = duration.as_secs() / 60;
        format!("{:02}:{:02}", minutes, seconds)
    }
}

impl UIComponent for ProgressBar {
    fn render(&self, context: &RenderContext) -> String {
        let bar_width = if self.show_time {
            context.width.saturating_sub(16) // Leave room for time display
        } else {
            context.width
        };
        
        let filled = if let Some(track_info) = &context.track_info {
            if let Some(duration) = track_info.duration {
                let progress = context.position.as_secs_f32() / duration.as_secs_f32();
                (progress * bar_width as f32).round() as usize
            } else {
                0
            }
        } else {
            0
        };
        
        let bar = format!(
            "[{}{}]",
            self.fill_char.to_string().repeat(filled.min(bar_width)),
            self.empty_char.to_string().repeat(bar_width.saturating_sub(filled))
        );
        
        if self.show_time {
            let current = Self::format_duration(&context.position);
            let total = context.track_info
                .as_ref()
                .and_then(|t| t.duration.as_ref())
                .map(Self::format_duration)
                .unwrap_or_else(|| "00:00".to_string());
            
            format!(" {} {}/{} ", bar, current, total)
        } else {
            format!(" {} ", bar)
        }
    }
    
    fn min_width(&self) -> usize {
        if self.show_time { 20 } else { 5 }
    }
}

/// Volume bar component
pub struct VolumeBar {
    fill_char: char,
    empty_char: char,
    show_percentage: bool,
}

impl Default for VolumeBar {
    fn default() -> Self {
        Self {
            fill_char: '/',
            empty_char: ' ',
            show_percentage: true,
        }
    }
}

impl VolumeBar {
    pub fn new() -> Self {
        Self::default()
    }
}

impl UIComponent for VolumeBar {
    fn render(&self, context: &RenderContext) -> String {
        let percentage = format!("{}%", (context.volume * 100.0).round() as u32);
        let bar_width = if self.show_percentage {
            context.width.saturating_sub(17) // Leave room for "volume: [] XXX%"
        } else {
            context.width.saturating_sub(10) // Just "volume: []"
        };
        
        let filled = (context.volume * bar_width as f32).round() as usize;
        
        let bar = format!(
            " volume: [{}{}]",
            self.fill_char.to_string().repeat(filled.min(bar_width)),
            self.empty_char.to_string().repeat(bar_width.saturating_sub(filled))
        );
        
        if self.show_percentage {
            format!("{} {:>4} ", bar, percentage)
        } else {
            format!("{} ", bar)
        }
    }
    
    fn min_width(&self) -> usize {
        if self.show_percentage { 20 } else { 12 }
    }
}

/// Status/Action bar component showing current track and playback state
pub struct StatusBar {
    show_bookmark_indicator: bool,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            show_bookmark_indicator: true,
        }
    }
}

impl UIComponent for StatusBar {
    fn render(&self, context: &RenderContext) -> String {
        let (status, track_name, width) = match context.playback_state {
            PlaybackState::Playing => {
                if let Some(track) = &context.track_info {
                    ("playing", Some(track.display_name.clone()), track.width)
                } else {
                    ("playing", None, 7)
                }
            }
            PlaybackState::Paused => {
                if let Some(track) = &context.track_info {
                    ("paused", Some(track.display_name.clone()), track.width)
                } else {
                    ("paused", None, 6)
                }
            }
            PlaybackState::Stopped => ("stopped", None, 7),
            PlaybackState::Loading => ("loading", None, 7),
        };
        
        let bookmark = if self.show_bookmark_indicator && context.is_bookmarked {
            "*"
        } else {
            ""
        };
        
        let full_text = if let Some(name) = track_name {
            format!("{} {}{}", status, bookmark, name.bold())
        } else {
            status.to_string()
        };
        
        let text_width = status.len() + bookmark.len() + width + if track_name.is_some() { 1 } else { 0 };
        
        if text_width > context.width {
            let truncated: String = full_text.graphemes(true).take(context.width - 3).collect();
            format!("{}...", truncated)
        } else {
            format!("{}{}", full_text, " ".repeat(context.width - text_width))
        }
    }
}

/// Control hint bar showing keyboard shortcuts
pub struct ControlBar {
    controls: Vec<(String, String)>,
}

impl Default for ControlBar {
    fn default() -> Self {
        Self {
            controls: vec![
                ("[s]".to_string(), "kip".to_string()),
                ("[p]".to_string(), "ause".to_string()),
                ("[q]".to_string(), "uit".to_string()),
            ],
        }
    }
}

impl ControlBar {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn with_controls(mut self, controls: Vec<(String, String)>) -> Self {
        self.controls = controls;
        self
    }
}

impl UIComponent for ControlBar {
    fn render(&self, context: &RenderContext) -> String {
        let total_len: usize = self.controls
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum();
        
        let spacing = if self.controls.len() > 1 {
            (context.width - total_len) / (self.controls.len() - 1)
        } else {
            0
        };
        
        let formatted: Vec<String> = self.controls
            .iter()
            .map(|(key, desc)| format!("{}{}", key.bold(), desc))
            .collect();
        
        let mut result = formatted.join(&" ".repeat(spacing));
        
        // Handle odd widths
        if context.width % 2 == 0 {
            result.push(' ');
        }
        
        result
    }
}

/// A vertical stack layout container
pub struct VStack {
    children: Vec<Box<dyn UIComponent>>,
    spacing: usize,
}

impl VStack {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            spacing: 0,
        }
    }
    
    pub fn with_spacing(mut self, spacing: usize) -> Self {
        self.spacing = spacing;
        self
    }
}

impl UIComponent for VStack {
    fn render(&self, context: &RenderContext) -> String {
        let mut lines = Vec::new();
        
        for child in &self.children {
            if child.is_visible() {
                lines.push(child.render(context));
                for _ in 0..self.spacing {
                    lines.push(String::new());
                }
            }
        }
        
        // Remove trailing spacing
        for _ in 0..self.spacing {
            lines.pop();
        }
        
        lines.join("\n")
    }
    
    fn height(&self) -> usize {
        self.children.iter()
            .filter(|c| c.is_visible())
            .map(|c| c.height())
            .sum::<usize>()
            + (self.spacing * self.children.len().saturating_sub(1))
    }
}

impl Container for VStack {
    fn add_child(&mut self, component: Box<dyn UIComponent>) {
        self.children.push(component);
    }
    
    fn remove_child(&mut self, index: usize) -> Option<Box<dyn UIComponent>> {
        if index < self.children.len() {
            Some(self.children.remove(index))
        } else {
            None
        }
    }
    
    fn child_count(&self) -> usize {
        self.children.len()
    }
    
    fn get_child_mut(&mut self, index: usize) -> Option<&mut dyn UIComponent> {
        self.children.get_mut(index).map(|b| &mut **b)
    }
}

/// A dynamic component that can switch between different states
pub struct DynamicComponent {
    current_state: usize,
    components: Vec<Box<dyn UIComponent>>,
}

impl DynamicComponent {
    pub fn new() -> Self {
        Self {
            current_state: 0,
            components: Vec::new(),
        }
    }
    
    pub fn add_state(&mut self, component: Box<dyn UIComponent>) -> usize {
        self.components.push(component);
        self.components.len() - 1
    }
    
    pub fn set_state(&mut self, state: usize) {
        if state < self.components.len() {
            self.current_state = state;
        }
    }
    
    pub fn current_state(&self) -> usize {
        self.current_state
    }
}

impl UIComponent for DynamicComponent {
    fn render(&self, context: &RenderContext) -> String {
        if let Some(component) = self.components.get(self.current_state) {
            component.render(context)
        } else {
            String::new()
        }
    }
    
    fn handle_event(&mut self, event: ComponentEvent) -> EventResult {
        if let Some(component) = self.components.get_mut(self.current_state) {
            component.handle_event(event)
        } else {
            EventResult::Ignored
        }
    }
    
    fn is_visible(&self) -> bool {
        self.components.get(self.current_state)
            .map(|c| c.is_visible())
            .unwrap_or(false)
    }
}

// ============= Component Factory =============

/// Factory for creating pre-configured components
pub struct ComponentFactory;

impl ComponentFactory {
    /// Create a default lowfi UI layout
    pub fn create_default_layout(minimalist: bool) -> Box<dyn UIComponent> {
        let mut stack = Box::new(VStack::new());
        
        // Status bar
        stack.add_child(Box::new(StatusBar::new()));
        
        // Progress/Volume bar (dynamic)
        let dynamic = Box::new(DynamicComponent::new());
        // We would add states to this dynamically based on timer
        stack.add_child(dynamic);
        
        // Control bar (if not minimalist)
        if !minimalist {
            stack.add_child(Box::new(ControlBar::new()));
        }
        
        stack as Box<dyn UIComponent>
    }
    
    /// Create a custom label
    pub fn label(text: impl Into<String>) -> Box<dyn UIComponent> {
        Box::new(Label::new(text))
    }
    
    /// Create a progress bar
    pub fn progress_bar() -> Box<dyn UIComponent> {
        Box::new(ProgressBar::new())
    }
    
    /// Create a volume bar
    pub fn volume_bar() -> Box<dyn UIComponent> {
        Box::new(VolumeBar::new())
    }
    
    /// Create a status bar
    pub fn status_bar() -> Box<dyn UIComponent> {
        Box::new(StatusBar::new())
    }
    
    /// Create a control bar with custom controls
    pub fn control_bar(controls: Vec<(String, String)>) -> Box<dyn UIComponent> {
        Box::new(ControlBar::new().with_controls(controls))
    }
}
