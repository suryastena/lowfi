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

use ui::{components, UIEvent};

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

    /// Cached width for progress bar updates
    progress_bar_width: usize,

    /// Whether the current track has been bookmarked
    bookmarked: AtomicBool,

    /// The [`TrackInfo`] of the current track
    current: ArcSwapOption<tracks::Info>,

    /// The tracks buffer
    tracks: RwLock<VecDeque<tracks::QueuedTrack>>,

    /// The actual list of tracks to be played
    list: List,

    /// The initial volume level
    volume: PersistentVolume,

    /// The web client
    client: Client,

    /// Keep the output stream handle alive
    _handle: OutputStreamHandle,

    /// Channel to emit progress updates for the UI
    progress_tx: watch::Sender<UIEvent>,
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

    /// Initializes the entire player, including audio devices & sink
    pub async fn new(args: &Args) -> eyre::Result<(Self, SendableOutputStream)> {
        // Compute UI progress bar width: full_width - 16
        let full_width = 21 + args.width.min(32) * 2;
        let progress_bar_width = full_width.saturating_sub(16);

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
            progress_bar_width,
            bookmarked: AtomicBool::new(false),
            current: ArcSwapOption::new(None),
            tracks: RwLock::new(VecDeque::with_capacity(args.buffer_size)),
            list,
            volume,
            client,
            _handle: handle,
            progress_tx,
        };

        Ok((player, SendableOutputStream(stream)))
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

        // Spawn progress emitter task that only notifies on actual bar change
        let mut progress_interval = interval(Duration::from_millis(250));
        let progress_tx = player.progress_tx.clone();
        let p = Arc::clone(&player);
        let mut last_bar = String::new();
        let progress_task = task::spawn(async move {
            loop {
                progress_interval.tick().await;
                // Compute new progress bar string
                let current_arc = p.current.load();
                let current_info = current_arc.as_ref();
                let new_bar = components::progress_bar(&p, current_info, p.progress_bar_width);
                if new_bar != last_bar {
                    last_bar = new_bar;
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
                    let _ = ui_tx.send(UIEvent::TrackChanged).await;
                    task::spawn(Self::next(
                        Arc::clone(&player),
                        itx.clone(),
                        tx.clone(),
                        debug,
                    ));
                }
                Messages::Play => {
                    player.sink.play();
                    let _ = ui_tx.send(UIEvent::PlaybackStateChanged).await;
                    #[cfg(feature = "mpris")]
                    mpris.playback(PlaybackStatus::Playing).await?;
                }
                Messages::Pause => {
                    player.sink.pause();
                    let _ = ui_tx.send(UIEvent::PlaybackStateChanged).await;
                    #[cfg(feature = "mpris")]
                    mpris.playback(PlaybackStatus::Paused).await?;
                }
                Messages::PlayPause => {
                    if player.sink.is_paused() {
                        player.sink.play();
                    } else {
                        player.sink.pause();
                    }
                    let _ = ui_tx.send(UIEvent::PlaybackStateChanged).await;
                    #[cfg(feature = "mpris")]
                    mpris
                        .playback(mpris.player().playback_status().await?)
                        .await?;
                }
                Messages::ChangeVolume(change) => {
                    player.set_volume(player.sink.volume() + change);
                    let _ = ui_tx.send(UIEvent::VolumeChanged).await;
                    #[cfg(feature = "mpris")]
                    mpris
                        .changed(vec![Property::Volume(player.sink.volume().into())])
                        .await?;
                }
                Messages::NewSong => {
                    new = true;
                    let _ = ui_tx.send(UIEvent::TrackChanged).await;
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
                    let _ = ui_tx.send(UIEvent::BookmarkChanged).await;
                }
                Messages::Quit => break,
            }
        }

        downloader.abort();
        progress_task.abort();

        Ok(())
    }
}
