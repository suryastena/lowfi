//! Example of how to create and use custom UI components
//! This demonstrates the extensibility of the modular component system

use std::time::Duration;
use crossterm::style::Stylize as _;

// Import the component system
use super::ui_component_system::*;

// ============= Custom Component Examples =============

/// A spectrum analyzer visualization component
pub struct SpectrumAnalyzer {
    bars: usize,
    heights: Vec<f32>,
    characters: Vec<char>,
}

impl SpectrumAnalyzer {
    pub fn new(bars: usize) -> Self {
        Self {
            bars,
            heights: vec![0.0; bars],
            characters: vec!['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'],
        }
    }
    
    pub fn update_spectrum(&mut self, data: Vec<f32>) {
        // Simulate spectrum data update
        self.heights = data.into_iter().take(self.bars).collect();
        while self.heights.len() < self.bars {
            self.heights.push(0.0);
        }
    }
}

impl UIComponent for SpectrumAnalyzer {
    fn render(&self, context: &RenderContext) -> String {
        let available_width = context.width;
        let bar_width = available_width / self.bars;
        
        let mut result = String::new();
        for height in &self.heights {
            let char_idx = (height * (self.characters.len() - 1) as f32) as usize;
            let char = self.characters[char_idx.min(self.characters.len() - 1)];
            result.push_str(&char.to_string().repeat(bar_width));
        }
        
        result
    }
    
    fn min_width(&self) -> usize {
        self.bars * 2
    }
    
    fn handle_event(&mut self, event: ComponentEvent) -> EventResult {
        match event {
            ComponentEvent::Custom(data) if data.starts_with("spectrum:") => {
                // Parse spectrum data from custom event
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

/// A playlist component showing upcoming tracks
pub struct PlaylistView {
    tracks: Vec<String>,
    current_index: usize,
    max_visible: usize,
}

impl PlaylistView {
    pub fn new(max_visible: usize) -> Self {
        Self {
            tracks: Vec::new(),
            current_index: 0,
            max_visible,
        }
    }
    
    pub fn set_tracks(&mut self, tracks: Vec<String>) {
        self.tracks = tracks;
    }
    
    pub fn set_current(&mut self, index: usize) {
        self.current_index = index;
    }
}

impl UIComponent for PlaylistView {
    fn render(&self, context: &RenderContext) -> String {
        if self.tracks.is_empty() {
            return "No tracks in playlist".to_string();
        }
        
        let start = self.current_index.saturating_sub(self.max_visible / 2);
        let end = (start + self.max_visible).min(self.tracks.len());
        
        let mut lines = Vec::new();
        for (i, track) in self.tracks[start..end].iter().enumerate() {
            let actual_index = start + i;
            let prefix = if actual_index == self.current_index {
                "▶ ".bold()
            } else {
                "  ".into()
            };
            
            let truncated = if track.len() > context.width - 3 {
                format!("{}...", &track[..context.width - 6])
            } else {
                track.clone()
            };
            
            lines.push(format!("{}{}", prefix, truncated));
        }
        
        lines.join("\n")
    }
    
    fn height(&self) -> usize {
        self.max_visible.min(self.tracks.len())
    }
}

/// A lyrics display component
pub struct LyricsDisplay {
    lyrics: Vec<(Duration, String)>,
    current_line: Option<usize>,
    show_timestamp: bool,
}

impl LyricsDisplay {
    pub fn new() -> Self {
        Self {
            lyrics: Vec::new(),
            current_line: None,
            show_timestamp: false,
        }
    }
    
    pub fn load_lyrics(&mut self, lyrics: Vec<(Duration, String)>) {
        self.lyrics = lyrics;
    }
    
    pub fn with_timestamps(mut self, show: bool) -> Self {
        self.show_timestamp = show;
        self
    }
}

impl UIComponent for LyricsDisplay {
    fn render(&self, context: &RenderContext) -> String {
        if self.lyrics.is_empty() {
            return "No lyrics available".italic().to_string();
        }
        
        // Find current line based on playback position
        let mut current = None;
        for (i, (time, _)) in self.lyrics.iter().enumerate() {
            if context.position >= *time {
                current = Some(i);
            } else {
                break;
            }
        }
        
        if let Some(idx) = current {
            let line = &self.lyrics[idx].1;
            if self.show_timestamp {
                let time = format_duration(&self.lyrics[idx].0);
                format!("[{}] {}", time, line.bold())
            } else {
                line.bold().to_string()
            }
        } else {
            "♪ ♪ ♪".to_string()
        }
    }
    
    fn handle_event(&mut self, event: ComponentEvent) -> EventResult {
        match event {
            ComponentEvent::ProgressUpdate(position, _) => {
                // Update current line based on position
                let mut new_line = None;
                for (i, (time, _)) in self.lyrics.iter().enumerate() {
                    if position >= *time {
                        new_line = Some(i);
                    } else {
                        break;
                    }
                }
                
                if new_line != self.current_line {
                    self.current_line = new_line;
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
        }
    }
}

fn format_duration(duration: &Duration) -> String {
    let seconds = duration.as_secs() % 60;
    let minutes = duration.as_secs() / 60;
    format!("{:02}:{:02}", minutes, seconds)
}

/// Network status indicator component
pub struct NetworkStatus {
    status: ConnectionStatus,
    show_details: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum ConnectionStatus {
    Connected,
    Buffering,
    Disconnected,
    Connecting,
}

impl NetworkStatus {
    pub fn new() -> Self {
        Self {
            status: ConnectionStatus::Connected,
            show_details: false,
        }
    }
    
    pub fn update_status(&mut self, status: ConnectionStatus) {
        self.status = status;
    }
}

impl UIComponent for NetworkStatus {
    fn render(&self, _context: &RenderContext) -> String {
        let (icon, text, style) = match self.status {
            ConnectionStatus::Connected => ("●", "Connected", "green"),
            ConnectionStatus::Buffering => ("◐", "Buffering", "yellow"),
            ConnectionStatus::Disconnected => ("○", "Disconnected", "red"),
            ConnectionStatus::Connecting => ("◑", "Connecting", "blue"),
        };
        
        if self.show_details {
            format!("{} {}", icon, text)
        } else {
            icon.to_string()
        }
    }
    
    fn min_width(&self) -> usize {
        if self.show_details { 15 } else { 1 }
    }
}

/// Equalizer preset selector component
pub struct EqualizerPreset {
    presets: Vec<String>,
    current: usize,
}

impl EqualizerPreset {
    pub fn new() -> Self {
        Self {
            presets: vec![
                "Flat".to_string(),
                "Bass Boost".to_string(),
                "Vocal".to_string(),
                "Classical".to_string(),
                "Rock".to_string(),
            ],
            current: 0,
        }
    }
    
    pub fn next_preset(&mut self) {
        self.current = (self.current + 1) % self.presets.len();
    }
    
    pub fn previous_preset(&mut self) {
        if self.current == 0 {
            self.current = self.presets.len() - 1;
        } else {
            self.current -= 1;
        }
    }
}

impl UIComponent for EqualizerPreset {
    fn render(&self, _context: &RenderContext) -> String {
        format!("EQ: {}", self.presets[self.current].bold())
    }
}

// ============= Advanced Layout Example =============

/// Create a complex UI layout with custom components
pub fn create_advanced_layout() -> Box<dyn UIComponent> {
    let mut main_layout = Box::new(VStack::new().with_spacing(1));
    
    // Top section with status and network indicator
    let mut top_bar = Box::new(HStack::new());
    top_bar.add_child(Box::new(StatusBar::new()));
    top_bar.add_child(Box::new(NetworkStatus::new()));
    main_layout.add_child(top_bar);
    
    // Middle section with visualizer
    main_layout.add_child(Box::new(SpectrumAnalyzer::new(20)));
    
    // Progress bar
    main_layout.add_child(Box::new(ProgressBar::new()));
    
    // Current lyrics line
    main_layout.add_child(Box::new(LyricsDisplay::new()));
    
    // Playlist view
    main_layout.add_child(Box::new(PlaylistView::new(3)));
    
    // Bottom controls with equalizer
    let mut bottom_bar = Box::new(HStack::new());
    bottom_bar.add_child(Box::new(ControlBar::new()));
    bottom_bar.add_child(Box::new(EqualizerPreset::new()));
    main_layout.add_child(bottom_bar);
    
    main_layout as Box<dyn UIComponent>
}

/// Horizontal stack layout (bonus component)
pub struct HStack {
    children: Vec<Box<dyn UIComponent>>,
}

impl HStack {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }
}

impl UIComponent for HStack {
    fn render(&self, context: &RenderContext) -> String {
        if self.children.is_empty() {
            return String::new();
        }
        
        let width_per_child = context.width / self.children.len();
        let mut parts = Vec::new();
        
        for child in &self.children {
            let mut child_context = context.clone();
            child_context.width = width_per_child;
            parts.push(child.render(&child_context));
        }
        
        parts.join(" ")
    }
}

impl Container for HStack {
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

// Need to implement Clone for RenderContext for the HStack implementation
impl Clone for RenderContext {
    fn clone(&self) -> Self {
        Self {
            width: self.width,
            playback_state: self.playback_state,
            track_info: self.track_info.clone(),
            volume: self.volume,
            position: self.position,
            is_bookmarked: self.is_bookmarked,
            borderless: self.borderless,
            custom_data: self.custom_data.clone(),
        }
    }
}

// ============= Usage Examples =============

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_custom_component_creation() {
        // Create a custom layout
        let layout = create_advanced_layout();
        
        // Create a render context
        let context = RenderContext {
            width: 80,
            playback_state: PlaybackState::Playing,
            track_info: Some(Arc::new(TrackInfo {
                name: "test_track".to_string(),
                display_name: "Test Track".to_string(),
                width: 10,
                duration: Some(Duration::from_secs(180)),
            })),
            volume: 0.75,
            position: Duration::from_secs(45),
            is_bookmarked: false,
            borderless: false,
            custom_data: std::collections::HashMap::new(),
        };
        
        // Render the layout
        let output = layout.render(&context);
        assert!(!output.is_empty());
    }
    
    #[test]
    fn test_dynamic_component_switching() {
        let mut dynamic = DynamicComponent::new();
        
        // Add different states
        let progress_idx = dynamic.add_state(Box::new(ProgressBar::new()));
        let volume_idx = dynamic.add_state(Box::new(VolumeBar::new()));
        let spectrum_idx = dynamic.add_state(Box::new(SpectrumAnalyzer::new(10)));
        
        // Switch between states
        dynamic.set_state(progress_idx);
        assert_eq!(dynamic.current_state(), progress_idx);
        
        dynamic.set_state(volume_idx);
        assert_eq!(dynamic.current_state(), volume_idx);
        
        dynamic.set_state(spectrum_idx);
        assert_eq!(dynamic.current_state(), spectrum_idx);
    }
}
