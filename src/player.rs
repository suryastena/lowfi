//! Responsible for playing & queueing audio
//! This also has the code for the underlying
//! audio server which adds new tracks.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use arc_swap::ArcSwapOption;
use downloader::Downloader;
use reqwest::Client;
use rodio::{OutputStream, OutputStreamHandle, Sink};
use tokio::{
    select,
    sync::{
        mpsc::{Receiver, Sender},
        watch, RwLock,
    },
    task,
    time::interval,
};

#[cfg(feature = "mpris")]
use mpris_server::{PlaybackStatus, PlayerInterface, Property};

use crate::{
    messages::Messages,
    play::{PersistentVolume, SendableOutputStream},
    tracks::{self, list::List},
    Args,
};

use ui::UIEvent;

pub mod audio;
pub mod bookmark;
pub mod downloader;
pub mod queue;
pub mod ui;

#[cfg(feature = "mpris")]
pub mod mpris;

/// The time to wait in between errors
const TIMEOUT: Duration = Duration::from_secs(3);

/// Main struct responsible for queuing up & playing tracks
pub struct Player {
    /// [rodio]'s [`Sink`] which can control playback
    pub sink: Sink,

    /// The internal buffer size
    pub buffer_size: usize,

    /// Whether the current track has been bookmarked
    pub bookmarked: AtomicBool,

    /// The [`TrackInfo`] of the current track
    pub current: ArcSwapOption<tracks::Info>,

    /// The tracks buffer
    pub tracks: RwLock<VecDeque<tracks::QueuedTrack>>,

    /// The actual list of tracks to be played
    pub list: List,

    /// The initial volume level
    pub volume: PersistentVolume,

    /// The web client
    pub client: Client,

    /// Keep the output stream handle alive
    _handle: OutputStreamHandle,

    /// Channel to emit progress updates for the UI
    /// This is now generic and can trigger any UI event
    progress_tx: watch::Sender<UIEvent>,

    /// Track if we should be emitting progress updates
    emit_progress: AtomicBool,
}

impl Player {
    /// Just a shorthand for setting `current`
    fn set_current(&self, info: tracks::Info) {
        self.current.store(Some(Arc::new(info)));
    }

    /// A shorthand for checking if `self.current` is [Some]
    pub fn current_exists(&self) -> bool {
        self.current.load().is_some()
    }

    /// Sets the volume of the sink, clamping 0.0..1.0
    pub fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume.clamp(0.0, 1.0));
    }

    /// Subscribe to progress update events
    pub fn subscribe_progress(&self) -> watch::Receiver<UIEvent> {
        self.progress_tx.subscribe()
    }

    /// Enable or disable progress emit
    pub fn set_progress_emit(&self, emit: bool) {
        self.emit_progress.store(emit, Ordering::Relaxed);
    }

    /// Initializes the entire player, including audio devices & sink
    pub async fn new(args: &Args) -> eyre::Result<(Self, SendableOutputStream)> {
        // Create watch channel for progress updates
        let (progress_tx, _) = watch::channel(UIEvent::ProgressUpdate);

        // Load the volume file
        let volume = PersistentVolume::load().await?;

        // Load the track list
        let list = List::load(args.track_list.as_ref()).await?;

        // Setup audio output stream
        #[cfg(target_os = "linux")]
        let (stream, handle) = if !args.alternate && !args.debug {
            audio::silent_get_output_stream()?
        } else {
            OutputStream::try_default()?
        };
        #[cfg(not(target_os = "linux"))]
        let (stream, handle) = OutputStream::try_default()?;

        let sink = Sink::try_new(&handle)?;
        if args.paused {
            sink.pause();
        }

        let client = Client::builder()
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .timeout(TIMEOUT)
            .build()?;

        let player = Self {
            sink,
            buffer_size: args.buffer_size,
            bookmarked: AtomicBool::new(false),
            current: ArcSwapOption::new(None),
            tracks: RwLock::new(VecDeque::with_capacity(args.buffer_size)),
            list,
            volume,
            client,
            _handle: handle,
            progress_tx,
            emit_progress: AtomicBool::new(true),
        };

        Ok((player, SendableOutputStream(stream)))
    }

    /// Helper to send UI events
    async fn send_ui_event(ui_tx: &Sender<UIEvent>, event: UIEvent) {
        let _ = ui_tx.send(event).await;
    }

    /// Get current playback info for UI components
    pub fn get_playback_info(&self) -> PlaybackInfo {
        PlaybackInfo {
            is_paused: self.sink.is_paused(),
            is_playing: self.current_exists() && !self.sink.is_paused(),
            volume: self.sink.volume(),
            position: self.sink.get_pos(),
            is_bookmarked: self.bookmarked.load(Ordering::Relaxed),
        }
    }

    /// This is the main "audio server"
    pub async fn play(
        player: Arc<Self>,
        tx: Sender<Messages>,
        mut rx: Receiver<Messages>,
        ui_tx: Sender<UIEvent>,
        debug: bool,
    ) -> eyre::Result<()> {
        #[cfg(feature = "mpris")]
        let mpris = mpris::Server::new(Arc::clone(&player), tx.clone())
            .await
            .inspect_err(|x| dbg!(x))?;

        let downloader = Downloader::new(Arc::clone(&player));
        let (itx, downloader) = downloader.start(debug);

        Downloader::notify(&itx).await?;
        player.set_volume(player.volume.float());

        // Spawn progress emitter task
        // This emits generic ProgressUpdate events that components can interpret
        let progress_interval_ms = 100;
        let mut progress_interval = interval(Duration::from_millis(progress_interval_ms));
        let progress_tx = player.progress_tx.clone();
        let p = Arc::clone(&player);
        let progress_task = task::spawn(async move {
            loop {
                progress_interval.tick().await;
                if p.emit_progress.load(Ordering::Relaxed)
                    && p.current_exists()
                    && !p.sink.is_paused()
                {
                    let _ = progress_tx.send(UIEvent::ProgressUpdate);
                }
            }
        });

        let mut new = false;
        loop {
            let clone = Arc::clone(&player);

            let msg = select! {
                biased;
                Some(x) = rx.recv() => x,
                Ok(()) = task::spawn_blocking(move || clone.sink.sleep_until_end()), if new => Messages::Next,
            };

            match msg {
                Messages::Next | Messages::Init | Messages::TryAgain => {
                    player.bookmarked.swap(false, Ordering::Relaxed);
                    new = false;
                    if msg == Messages::Next && !player.current_exists() {
                        continue;
                    }
                    Self::send_ui_event(&ui_tx, UIEvent::TrackChanged).await;
                    task::spawn(Self::next(
                        Arc::clone(&player),
                        itx.clone(),
                        tx.clone(),
                        debug,
                    ));
                }
                Messages::Play => {
                    player.sink.play();
                    Self::send_ui_event(&ui_tx, UIEvent::PlaybackStateChanged).await;
                    #[cfg(feature = "mpris")]
                    mpris.playback(PlaybackStatus::Playing).await?;
                }
                Messages::Pause => {
                    player.sink.pause();
                    Self::send_ui_event(&ui_tx, UIEvent::PlaybackStateChanged).await;
                    #[cfg(feature = "mpris")]
                    mpris.playback(PlaybackStatus::Paused).await?;
                }
                Messages::PlayPause => {
                    if player.sink.is_paused() {
                        player.sink.play();
                    } else {
                        player.sink.pause();
                    }
                    Self::send_ui_event(&ui_tx, UIEvent::PlaybackStateChanged).await;
                    #[cfg(feature = "mpris")]
                    mpris
                        .playback(mpris.player().playback_status().await?)
                        .await?;
                }
                Messages::ChangeVolume(change) => {
                    player.set_volume(player.sink.volume() + change);
                    Self::send_ui_event(&ui_tx, UIEvent::VolumeChanged).await;
                    #[cfg(feature = "mpris")]
                    mpris
                        .changed(vec![Property::Volume(player.sink.volume().into())])
                        .await?;
                }
                Messages::NewSong => {
                    new = true;
                    Self::send_ui_event(&ui_tx, UIEvent::TrackChanged).await;
                    #[cfg(feature = "mpris")]
                    mpris
                        .changed(vec![
                            Property::Metadata(mpris.player().metadata().await?),
                            Property::PlaybackStatus(mpris.player().playback_status().await?),
                        ])
                        .await?;
                    continue;
                }
                Messages::Bookmark => {
                    let loaded = player.current.load();
                    let current = loaded.as_ref().unwrap().clone();
                    let bookmarked = bookmark::bookmark(
                        current.full_path.clone(),
                        if current.custom_name {
                            Some(current.display_name.clone())
                        } else {
                            None
                        },
                    )
                    .await?;
                    player.bookmarked.swap(bookmarked, Ordering::Relaxed);
                    Self::send_ui_event(&ui_tx, UIEvent::BookmarkChanged).await;
                }
                Messages::Quit => break,
            }
        }

        downloader.abort();
        progress_task.abort();

        Ok(())
    }
}

/// Playback information for UI components
#[derive(Debug, Clone, Copy)]
pub struct PlaybackInfo {
    pub is_paused: bool,
    pub is_playing: bool,
    pub volume: f32,
    pub position: Duration,
    pub is_bookmarked: bool,
}
