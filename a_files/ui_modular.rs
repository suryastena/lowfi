//! Modular UI implementation using the component system
//! This replaces the existing ui.rs with a more flexible architecture

use std::{
    io::{stdout, Stdout},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use crossterm::{
    cursor::{Hide, MoveTo, MoveToColumn, MoveUp, Show},
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    style::Print,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use lazy_static::lazy_static;
use thiserror::Error;
use tokio::{
    sync::{
        mpsc::{Receiver, Sender},
        watch,
    },
    task,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{Messages, Player};

// Import our component system
mod components;
use components::*;

/// The error type for the UI
#[derive(Debug, Error)]
pub enum UIError {
    #[error("unable to convert number")]
    Conversion(#[from] std::num::TryFromIntError),

    #[error("unable to write output")]
    Write(#[from] std::io::Error),

    #[error("sending message to backend failed")]
    Communication(#[from] tokio::sync::mpsc::error::SendError<Messages>),

    #[error("failed to send UI event")]
    UiSend(#[from] tokio::sync::mpsc::error::SendError<UIEvent>),
}

/// Events that trigger UI updates
#[derive(Debug, Clone)]
pub enum UIEvent {
    /// Redraw the entire UI
    Redraw,
    /// Volume changed
    VolumeChanged,
    /// Track changed
    TrackChanged,
    /// Playback state changed (play/pause)
    PlaybackStateChanged,
    /// Progress update (for progress bar)
    ProgressUpdate,
    /// Bookmark state changed
    BookmarkChanged,
}

/// How long the audio bar will be visible for when audio is adjusted
const AUDIO_BAR_DURATION: usize = 10;

lazy_static! {
    /// The volume timer
    static ref VOLUME_TIMER: AtomicUsize = AtomicUsize::new(0);
}

/// Sets the volume timer to trigger the audio display
pub fn flash_audio() {
    VOLUME_TIMER.store(1, Ordering::Relaxed);
}

/// Enhanced window manager with component support
pub struct ComponentWindow {
    /// The root component to render
    root: Box<dyn UIComponent>,
    /// Whether or not to include borders
    borderless: bool,
    /// Pre-rendered borders
    borders: [String; 2],
    /// Window width
    width: usize,
    /// The output stream
    out: Stdout,
    /// Render context that gets updated
    context: RenderContext,
}

impl ComponentWindow {
    /// Create a new component-based window
    pub fn new(width: usize, borderless: bool) -> Self {
        let borders = if borderless {
            [String::new(), String::new()]
        } else {
            let middle = "─".repeat(width + 2);
            [format!("┌{middle}┐"), format!("└{middle}┘")]
        };

        let context = RenderContext {
            width,
            playback_state: PlaybackState::Loading,
            track_info: None,
            volume: 1.0,
            position: Duration::new(0, 0),
            is_bookmarked: false,
            borderless,
            custom_data: std::collections::HashMap::new(),
        };

        Self {
            root: ComponentFactory::create_default_layout(false),
            borders,
            borderless,
            width,
            out: stdout(),
            context,
        }
    }

    /// Set the root component
    pub fn set_root(&mut self, component: Box<dyn UIComponent>) {
        self.root = component;
    }

    /// Update the render context
    pub fn update_context<F>(&mut self, updater: F)
    where
        F: FnOnce(&mut RenderContext),
    {
        updater(&mut self.context);
    }

    /// Render the window with the current component tree
    pub fn render(&mut self) -> eyre::Result<(), UIError> {
        let rendered_content = self.root.render(&self.context);
        let lines: Vec<String> = rendered_content.lines().map(String::from).collect();
        
        self.draw(lines, true)
    }

    /// Draw the window with borders and proper formatting
    fn draw(&mut self, content: Vec<String>, space: bool) -> eyre::Result<(), UIError> {
        let len: u16 = content.len().try_into()?;

        let menu: String = content.into_iter().fold(String::new(), |mut output, x| {
            let padding = if self.borderless { " " } else { "│" };
            let space = if space {
                " ".repeat(self.width.saturating_sub(x.graphemes(true).count()))
            } else {
                String::new()
            };
            
            use std::fmt::Write;
            write!(output, "{padding} {x}{space} {padding}\r\n").unwrap();
            output
        });

        #[cfg(windows)]
        let (height, suffix) = (len + 2, "\r\n");
        #[cfg(not(windows))]
        let (height, suffix) = (len + 1, "");

        let rendered = format!("{}\r\n{menu}{}{suffix}", self.borders[0], self.borders[1]);

        crossterm::execute!(
            self.out,
            Clear(ClearType::FromCursorDown),
            MoveToColumn(0),
            Print(rendered),
            MoveToColumn(0),
            MoveUp(height),
        )?;

        Ok(())
    }
}

/// Main UI manager that coordinates components and state
pub struct UIManager {
    window: ComponentWindow,
    player: Arc<Player>,
    minimalist: bool,
    /// Dynamic component for switching between progress and volume
    middle_component: DynamicComponent,
    progress_bar_idx: usize,
    volume_bar_idx: usize,
}

impl UIManager {
    pub fn new(player: Arc<Player>, width: usize, borderless: bool, minimalist: bool) -> Self {
        let mut window = ComponentWindow::new(width, borderless);
        
        // Create the component layout
        let mut layout = VStack::new();
        
        // Status bar
        layout.add_child(Box::new(StatusBar::new()));
        
        // Dynamic middle component (switches between progress and volume)
        let mut middle = DynamicComponent::new();
        let progress_idx = middle.add_state(Box::new(ProgressBar::new()));
        let volume_idx = middle.add_state(Box::new(VolumeBar::new()));
        middle.set_state(progress_idx);
        
        layout.add_child(Box::new(middle.clone()));
        
        // Control bar (if not minimalist)
        if !minimalist {
            layout.add_child(Box::new(ControlBar::new()));
        }
        
        window.set_root(Box::new(layout));
        
        Self {
            window,
            player,
            minimalist,
            middle_component: middle,
            progress_bar_idx: progress_idx,
            volume_bar_idx: volume_idx,
        }
    }
    
    /// Update the UI based on current player state
    pub fn update(&mut self) -> eyre::Result<(), UIError> {
        // Load current track info
        let current = self.player.current.load();
        let current_ref = current.as_ref();
        
        // Update render context
        self.window.update_context(|ctx| {
            // Update playback state
            ctx.playback_state = if current_ref.is_none() {
                PlaybackState::Loading
            } else if self.player.sink.is_paused() {
                PlaybackState::Paused
            } else {
                PlaybackState::Playing
            };
            
            // Update track info
            ctx.track_info = current_ref.map(|info| {
                Arc::new(TrackInfo {
                    name: info.name.clone(),
                    display_name: info.display_name.clone(),
                    width: info.width,
                    duration: info.duration,
                })
            });
            
            // Update volume
            ctx.volume = self.player.sink.volume();
            
            // Update position
            ctx.position = if current_ref.is_some() {
                self.player.sink.get_pos()
            } else {
                Duration::new(0, 0)
            };
            
            // Update bookmark status
            ctx.is_bookmarked = self.player.bookmarked.load(Ordering::Relaxed);
        });
        
        // Handle volume timer for dynamic switching
        let timer = VOLUME_TIMER.load(Ordering::Relaxed);
        
        if timer > 0 {
            // Show volume bar
            self.middle_component.set_state(self.volume_bar_idx);
            
            if timer <= AUDIO_BAR_DURATION {
                VOLUME_TIMER.fetch_add(1, Ordering::Relaxed);
            } else {
                VOLUME_TIMER.store(0, Ordering::Relaxed);
            }
        } else {
            // Show progress bar
            self.middle_component.set_state(self.progress_bar_idx);
        }
        
        // Render the window
        self.window.render()
    }
    
    /// Handle a UI event
    pub fn handle_event(&mut self, event: UIEvent) -> eyre::Result<(), UIError> {
        match event {
            UIEvent::Redraw
            | UIEvent::VolumeChanged
            | UIEvent::TrackChanged
            | UIEvent::PlaybackStateChanged
            | UIEvent::BookmarkChanged => {
                self.update()?;
            }
            UIEvent::ProgressUpdate => {
                if !self.player.sink.is_paused() && self.player.current_exists() {
                    self.update()?;
                }
            }
        }
        Ok(())
    }
}

/// Terminal environment manager
pub struct Environment {
    enhancement: bool,
    alternate: bool,
}

impl Environment {
    pub fn ready(alternate: bool) -> eyre::Result<Self, UIError> {
        let mut lock = stdout().lock();

        crossterm::execute!(lock, Hide)?;

        if alternate {
            crossterm::execute!(lock, EnterAlternateScreen, MoveTo(0, 0))?;
        }

        terminal::enable_raw_mode()?;
        let enhancement = terminal::supports_keyboard_enhancement()?;

        if enhancement {
            crossterm::execute!(
                lock,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }

        Ok(Self {
            enhancement,
            alternate,
        })
    }

    pub fn cleanup(&self) -> eyre::Result<(), UIError> {
        let mut lock = stdout().lock();

        if self.alternate {
            crossterm::execute!(lock, LeaveAlternateScreen)?;
        }

        crossterm::execute!(lock, Clear(ClearType::FromCursorDown), Show)?;

        if self.enhancement {
            crossterm::execute!(lock, PopKeyboardEnhancementFlags)?;
        }

        terminal::disable_raw_mode()?;
        eprintln!("bye! :)");

        Ok(())
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// Main interface loop using the component system
async fn interface_loop(
    player: Arc<Player>,
    minimalist: bool,
    borderless: bool,
    width: usize,
    mut ui_rx: Receiver<UIEvent>,
    mut progress_rx: watch::Receiver<UIEvent>,
) -> eyre::Result<(), UIError> {
    let mut ui_manager = UIManager::new(player, width, borderless, minimalist);
    
    // Initial render
    ui_manager.update()?;
    
    loop {
        tokio::select! {
            Some(event) = ui_rx.recv() => {
                ui_manager.handle_event(event)?;
            }
            
            Ok(_) = progress_rx.changed() => {
                ui_manager.handle_event(UIEvent::ProgressUpdate)?;
            }
        }
    }
}

/// Start the modular UI system
pub async fn start(
    player: Arc<Player>,
    sender: Sender<Messages>,
    args: crate::Args,
    mut ui_rx: Receiver<UIEvent>,
    progress_rx: watch::Receiver<UIEvent>,
) -> eyre::Result<(), UIError> {
    let environment = Environment::ready(args.alternate)?;
    
    // Create UI event channel for input
    let (ui_tx_input, mut ui_rx_input) = tokio::sync::mpsc::channel(100);
    
    // Merge event streams
    let (ui_tx_merged, ui_rx_merged) = tokio::sync::mpsc::channel(100);
    
    let merge_task = task::spawn(async move {
        loop {
            tokio::select! {
                Some(event) = ui_rx.recv() => {
                    let _ = ui_tx_merged.send(event).await;
                }
                Some(event) = ui_rx_input.recv() => {
                    let _ = ui_tx_merged.send(event).await;
                }
            }
        }
    });
    
    let interface = task::spawn(interface_loop(
        Arc::clone(&player),
        args.minimalist,
        args.borderless,
        21 + args.width.min(32) * 2,
        ui_rx_merged,
        progress_rx,
    ));
    
    // Reuse the existing input module
    super::input::listen(sender, ui_tx_input).await?;
    
    merge_task.abort();
    interface.abort();
    
    environment.cleanup()?;
    
    Ok(())
}
