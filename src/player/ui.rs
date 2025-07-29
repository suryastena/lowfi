//! Modular UI implementation using the component system

use std::{
    io::{stdout, Stdout},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use crossterm::{
    cursor::{Hide, MoveToColumn, MoveUp, Show},
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    style::Print,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use lazy_static::lazy_static;
use thiserror::Error;
use tokio::{
    sync::{mpsc::{Receiver, Sender}, watch},
    task,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{Messages, Player};

// Import our component system
mod components;
use components::*;

/// Allow shared, thread-safe ownership of DynamicComponent between UIManager and layout
impl UIComponent for Arc<Mutex<DynamicComponent>> {
    fn render(&self, context: &RenderContext) -> String {
        let component = self.lock().unwrap();
        component.render(context)
    }
}

pub mod input;


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
    Redraw,
    VolumeChanged,
    TrackChanged,
    PlaybackStateChanged,
    ProgressUpdate,
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
    root: Box<dyn UIComponent>,
    borderless: bool,
    borders: [String; 2],
    width: usize,
    out: Stdout,
    context: RenderContext,
}

impl ComponentWindow {
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
        Self { root: ComponentFactory::create_default_layout(false), borders, borderless, width, out: stdout(), context }
    }

    pub fn set_root(&mut self, component: Box<dyn UIComponent>) {
        self.root = component
    }

    pub fn update_context<F>(&mut self, updater: F)
    where
        F: FnOnce(&mut RenderContext),
    {
        updater(&mut self.context)
    }

    pub fn render(&mut self) -> eyre::Result<(), UIError> {
        let rendered_content = self.root.render(&self.context);
        let lines: Vec<String> = rendered_content.lines().map(String::from).collect();
        self.draw(lines, true)
    }

    fn draw(&mut self, content: Vec<String>, space: bool) -> eyre::Result<(), UIError> {
        let len: u16 = content.len().try_into()?;
        let menu = content.into_iter().fold(String::new(), |mut output, x| {
            let padding = if self.borderless { " " } else { "│" };
            let filler = if space {
                " ".repeat(self.width.saturating_sub(x.graphemes(true).count()))
            } else {
                String::new()
            };
            use std::fmt::Write;
            write!(output, "{padding} {x}{filler} {padding}\r\n").unwrap();
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
    middle_component: Arc<Mutex<DynamicComponent>>,
    progress_bar_idx: usize,
    volume_bar_idx: usize,
}

impl UIManager {
    pub fn new(
        player: Arc<Player>,
        width: usize,
        borderless: bool,
        minimalist: bool,
    ) -> Self {
        let mut window = ComponentWindow::new(width, borderless);
        let mut layout = VStack::new();

        layout.add_child(Box::new(StatusBar::new()));

        // Dynamic middle component
        let middle = Arc::new(Mutex::new(DynamicComponent::new()));
        let (progress_idx, volume_idx) = {
            let mut mid = middle.lock().unwrap();
            let p = mid.add_state(Box::new(ProgressBar::new()));
            let v = mid.add_state(Box::new(VolumeBar::new()));
            mid.set_state(p);
            (p, v)
        };
        layout.add_child(Box::new(Arc::clone(&middle)));

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

    pub fn update(&mut self) -> eyre::Result<(), UIError> {
        let current = self.player.current.load();
        let current_ref = current.as_ref();

        self.window.update_context(|ctx| {
            ctx.playback_state = if current_ref.is_none() {
                PlaybackState::Loading
            } else if self.player.sink.is_paused() {
                PlaybackState::Paused
            } else {
                PlaybackState::Playing
            };

            ctx.track_info = current_ref.map(|info| {
                Arc::new(TrackInfo {
                    name: info.display_name.clone(),
                    display_name: info.display_name.clone(),
                    width: info.width,
                    duration: info.duration,
                })
            });

            ctx.volume = self.player.sink.volume();
            ctx.position = current_ref
                .map_or(Duration::new(0, 0), |_| self.player.sink.get_pos());
            ctx.is_bookmarked = self.player.bookmarked.load(Ordering::Relaxed);
        });

        let timer = VOLUME_TIMER.load(Ordering::Relaxed);

        if timer > 0 {
            let mut mid = self.middle_component.lock().unwrap();
            mid.set_state(self.volume_bar_idx);

            if timer <= AUDIO_BAR_DURATION {
                VOLUME_TIMER.fetch_add(1, Ordering::Relaxed);
            } else {
                VOLUME_TIMER.store(0, Ordering::Relaxed);
            }
        } else {
            let mut mid = self.middle_component.lock().unwrap();
            mid.set_state(self.progress_bar_idx);
        }

        self.window.render()
    }

    pub fn handle_event(&mut self, event: UIEvent) -> eyre::Result<(), UIError> {
        match event {
            UIEvent::Redraw
            | UIEvent::VolumeChanged
            | UIEvent::TrackChanged
            | UIEvent::PlaybackStateChanged
            | UIEvent::BookmarkChanged => {
                self.update()?;
            }
            UIEvent::ProgressUpdate if !self.player.sink.is_paused() && self.player.current_exists() => {
                self.update()?;
            }
            _ => {}
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
            crossterm::execute!(lock, EnterAlternateScreen, MoveToColumn(0))?;
        }
        terminal::enable_raw_mode()?;
        let enhancement = terminal::supports_keyboard_enhancement()?;
        if enhancement {
            crossterm::execute!(
                lock,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }
        Ok(Self { enhancement, alternate })
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
    ui_manager.update()?;
    loop {
        tokio::select! {
            Some(event) = ui_rx.recv() => ui_manager.handle_event(event)?,
            Ok(_) = progress_rx.changed() => ui_manager.handle_event(UIEvent::ProgressUpdate)?,
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
    let (ui_tx_input, mut ui_rx_input) = tokio::sync::mpsc::channel(100);
    let (ui_tx_merged, ui_rx_merged) = tokio::sync::mpsc::channel(100);
    let merge_task = task::spawn(async move {
        loop {
            tokio::select! {
                Some(event) = ui_rx.recv() => { let _ = ui_tx_merged.send(event).await; }
                Some(event) = ui_rx_input.recv() => { let _ = ui_tx_merged.send(event).await; }
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
    input::listen(sender, ui_tx_input).await?;
    merge_task.abort();
    interface.abort();
    environment.cleanup()?;
    Ok(())
}