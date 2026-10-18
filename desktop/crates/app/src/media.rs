//! Playback for the open day: audio files and video sound through a native
//! backend, at most one medium at a time. The editor never touches the OS:
//! it emits [`dainiki_editor::EditorEvent::MediaToggle`] and paints whatever
//! [`dainiki_editor::MediaPlaybackState`] the app pushes back.
//!
//! The macOS backend is `AVAudioPlayer`; other platforms report a playback
//! error until their backend lands (video frames arrive in the same shape).

use std::path::Path;
use std::time::Duration;

use dainiki_editor::{MediaKind, MediaPlaybackState};

/// A backend decodes and plays one audio stream at a time. Video files use
/// the same path for their sound; their frames ride a separate pump.
pub trait AudioBackend {
    /// Loads `path` as the current stream, replacing anything loaded.
    fn load(&mut self, path: &Path) -> Result<(), String>;
    fn play(&mut self) -> Result<(), String>;
    fn pause(&mut self);
    fn stop(&mut self);
    /// Returns the stream to its start (a replay after the end).
    fn rewind(&mut self);
    fn position(&self) -> Duration;
    fn duration(&self) -> Option<Duration>;
    fn playing(&self) -> bool;
}

/// One loaded medium.
struct Current {
    destination: String,
    resolved: std::path::PathBuf,
    kind: MediaKind,
    playing: bool,
    error: Option<String>,
}

/// The day's player: toggling semantics, state snapshots, and the platform
/// backend behind a trait so the shell is testable without audio devices.
pub struct Player {
    backend: Box<dyn AudioBackend>,
    current: Option<Current>,
}

impl Player {
    pub fn new(backend: Box<dyn AudioBackend>) -> Self {
        Self {
            backend,
            current: None,
        }
    }

    /// The player with the platform's real backend.
    pub fn platform() -> Self {
        Self::new(platform_backend())
    }

    /// Toggles `resolved`: same medium → pause/resume; different → replace.
    /// Returns the state the editor should show now.
    pub fn toggle(
        &mut self,
        destination: &str,
        resolved: &Path,
        kind: MediaKind,
    ) -> MediaPlaybackState {
        let same = self
            .current
            .as_ref()
            .is_some_and(|current| current.resolved == resolved);
        if same {
            if let Some(current) = self.current.as_mut() {
                if current.playing {
                    self.backend.pause();
                    current.playing = false;
                } else {
                    // A finished stream replays from its start.
                    if let Some(duration) = self.backend.duration()
                        && self.backend.position() >= duration
                    {
                        self.backend.rewind();
                    }
                    match self.backend.play() {
                        Ok(()) => {
                            current.playing = true;
                            current.error = None;
                        }
                        Err(error) => current.error = Some(error),
                    }
                }
            }
        } else {
            self.backend.stop();
            let mut current = Current {
                destination: destination.to_string(),
                resolved: resolved.to_path_buf(),
                kind,
                playing: false,
                error: None,
            };
            match self
                .backend
                .load(resolved)
                .and_then(|()| self.backend.play())
            {
                Ok(()) => current.playing = true,
                Err(error) => current.error = Some(error),
            }
            self.current = Some(current);
        }
        self.state_for_current()
    }

    /// Stops playback and forgets the medium.
    pub fn stop(&mut self) {
        self.backend.stop();
        self.current = None;
    }

    /// The kind of the loaded medium, for messages and host-side dispatch
    /// (video frames ride their own pump).
    pub fn kind(&self) -> Option<MediaKind> {
        self.current.as_ref().map(|current| current.kind)
    }

    /// Whether the tick loop should keep running.
    pub fn playing(&self) -> bool {
        self.current.as_ref().is_some_and(|current| current.playing)
    }

    /// The current state, after folding in a natural end (the backend stopped
    /// on its own).
    pub fn state(&mut self) -> Option<(String, MediaPlaybackState)> {
        if let Some(current) = self.current.as_mut()
            && current.playing
            && !self.backend.playing()
        {
            current.playing = false;
        }
        let destination = self.current.as_ref()?.destination.clone();
        Some((destination, self.state_for_current()))
    }

    fn state_for_current(&self) -> MediaPlaybackState {
        let Some(current) = &self.current else {
            return MediaPlaybackState::default();
        };
        MediaPlaybackState {
            playing: current.playing && self.backend.playing(),
            position_ms: self.backend.position().as_millis() as u64,
            duration_ms: self.backend.duration().map(|d| d.as_millis() as u64),
            error: current.error.clone(),
        }
    }
}

#[cfg(target_os = "macos")]
fn platform_backend() -> Box<dyn AudioBackend> {
    Box::new(mac::MacAudio::default())
}

#[cfg(not(target_os = "macos"))]
fn platform_backend() -> Box<dyn AudioBackend> {
    Box::new(Unsupported)
}

/// A backend that reports playback as unavailable.
#[cfg(not(target_os = "macos"))]
struct Unsupported;

#[cfg(not(target_os = "macos"))]
impl AudioBackend for Unsupported {
    fn load(&mut self, _path: &Path) -> Result<(), String> {
        Err("playback is not implemented on this platform yet".into())
    }
    fn play(&mut self) -> Result<(), String> {
        Err("playback is not implemented on this platform yet".into())
    }
    fn pause(&mut self) {}
    fn stop(&mut self) {}
    fn rewind(&mut self) {}
    fn position(&self) -> Duration {
        Duration::ZERO
    }
    fn duration(&self) -> Option<Duration> {
        None
    }
    fn playing(&self) -> bool {
        false
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::path::Path;
    use std::time::Duration;

    use objc2::AllocAnyThread as _;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2_avf_audio::AVAudioPlayer;
    use objc2_foundation::{NSError, NSString, NSURL};

    use super::AudioBackend;

    /// AVAudioPlayer handles mp3, m4a/aac, wav, caf, aiff and more, and plays
    /// the sound track of a video file unchanged.
    #[derive(Default)]
    pub struct MacAudio {
        player: Option<Retained<AVAudioPlayer>>,
    }

    fn ns_url(path: &Path) -> Result<Retained<NSURL>, String> {
        let text = path
            .to_str()
            .ok_or_else(|| "the file path is not valid UTF-8".to_string())?;
        let text = NSString::from_str(text);
        Ok(NSURL::fileURLWithPath(&text))
    }

    fn describe(error: &NSError) -> String {
        error.localizedDescription().to_string()
    }

    impl AudioBackend for MacAudio {
        fn load(&mut self, path: &Path) -> Result<(), String> {
            self.player = None;
            let url = ns_url(path)?;
            let player = autoreleasepool(|_| unsafe {
                AVAudioPlayer::initWithContentsOfURL_error(AVAudioPlayer::alloc(), &url)
            })
            .map_err(|error| describe(&error))?;
            unsafe { player.prepareToPlay() };
            self.player = Some(player);
            Ok(())
        }

        fn play(&mut self) -> Result<(), String> {
            let Some(player) = &self.player else {
                return Err("no medium is loaded".into());
            };
            if unsafe { player.play() } {
                Ok(())
            } else {
                Err("the file could not be played".into())
            }
        }

        fn pause(&mut self) {
            if let Some(player) = &self.player {
                unsafe { player.pause() };
            }
        }

        fn stop(&mut self) {
            if let Some(player) = &self.player {
                unsafe { player.stop() };
            }
            self.player = None;
        }

        fn rewind(&mut self) {
            if let Some(player) = &self.player {
                unsafe { player.setCurrentTime(0.0) };
            }
        }

        fn position(&self) -> Duration {
            self.player
                .as_ref()
                .map(|player| Duration::from_secs_f64(unsafe { player.currentTime() }.max(0.0)))
                .unwrap_or_default()
        }

        fn duration(&self) -> Option<Duration> {
            self.player
                .as_ref()
                .map(|player| Duration::from_secs_f64(unsafe { player.duration() }.max(0.0)))
        }

        fn playing(&self) -> bool {
            self.player
                .as_ref()
                .is_some_and(|player| unsafe { player.isPlaying() })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::time::Duration;

    use dainiki_editor::MediaKind;

    use super::{AudioBackend, Player};

    /// A scripted backend: no decode, full control.
    #[derive(Default)]
    struct FakeAudio {
        state: Rc<FakeState>,
    }

    #[derive(Default)]
    struct FakeState {
        loaded: std::cell::RefCell<Option<PathBuf>>,
        playing: Cell<bool>,
        position: Cell<u64>,
        duration: Cell<Option<u64>>,
        rewind_calls: Cell<u32>,
        load_error: Cell<Option<String>>,
        play_error: Cell<Option<String>>,
    }

    impl AudioBackend for FakeAudio {
        fn load(&mut self, path: &Path) -> Result<(), String> {
            self.state.loaded.replace(Some(path.to_path_buf()));
            self.state.position.set(0);
            match self.state.load_error.take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
        fn play(&mut self) -> Result<(), String> {
            match self.state.play_error.take() {
                Some(error) => Err(error),
                None => {
                    self.state.playing.set(true);
                    Ok(())
                }
            }
        }
        fn pause(&mut self) {
            self.state.playing.set(false);
        }
        fn stop(&mut self) {
            self.state.playing.set(false);
            self.state.loaded.replace(None);
            self.state.position.set(0);
        }
        fn rewind(&mut self) {
            self.state
                .rewind_calls
                .set(self.state.rewind_calls.get() + 1);
            self.state.position.set(0);
        }
        fn position(&self) -> Duration {
            Duration::from_millis(self.state.position.get())
        }
        fn duration(&self) -> Option<Duration> {
            self.state.duration.get().map(Duration::from_millis)
        }
        fn playing(&self) -> bool {
            self.state.playing.get()
        }
    }

    fn setup() -> (Player, Rc<FakeState>) {
        let state = Rc::new(FakeState::default());
        let player = Player::new(Box::new(FakeAudio {
            state: state.clone(),
        }));
        (player, state)
    }

    #[test]
    fn toggling_a_new_medium_loads_and_plays_it() {
        let (mut player, state) = setup();
        let path = PathBuf::from("/journal/assets/take.mp3");
        let playback_state = player.toggle("assets/take.mp3", &path, MediaKind::Audio);
        assert!(playback_state.playing, "a fresh medium starts playing");
        assert_eq!(state.loaded.borrow().as_deref(), Some(path.as_path()));
        assert!(player.playing());
    }

    #[test]
    fn toggling_the_same_medium_pauses_and_resumes() {
        let (mut player, state) = setup();
        let path = PathBuf::from("/journal/assets/take.mp3");
        player.toggle("assets/take.mp3", &path, MediaKind::Audio);

        state.position.set(900);
        let paused = player.toggle("assets/take.mp3", &path, MediaKind::Audio);
        assert!(!paused.playing, "the second toggle pauses");
        assert!(!state.playing.get());

        let resumed = player.toggle("assets/take.mp3", &path, MediaKind::Audio);
        assert!(resumed.playing, "the third toggle resumes");
        assert!(resumed.position_ms >= 900, "position survives the pause");
    }

    #[test]
    fn a_finished_medium_replays_from_the_start() {
        let (mut player, state) = setup();
        let path = PathBuf::from("/journal/assets/take.mp3");
        state.duration.set(Some(1_000));
        player.toggle("assets/take.mp3", &path, MediaKind::Audio);

        // Natural end: the backend stopped on its own.
        state.playing.set(false);
        state.position.set(1_000);
        let ended = player.state().expect("still current").1;
        assert!(!ended.playing, "a natural end shows as paused");

        state.duration.set(Some(1_000));
        player.toggle("assets/take.mp3", &path, MediaKind::Audio);
        assert_eq!(state.rewind_calls.get(), 1, "replays from the start");
    }

    #[test]
    fn toggling_another_medium_replaces_the_first() {
        let (mut player, state) = setup();
        player.toggle(
            "assets/one.mp3",
            Path::new("/j/assets/one.mp3"),
            MediaKind::Audio,
        );
        player.toggle(
            "assets/two.mp3",
            Path::new("/j/assets/two.mp3"),
            MediaKind::Audio,
        );
        assert_eq!(
            state.loaded.borrow().as_deref(),
            Some(Path::new("/j/assets/two.mp3"))
        );
        let (destination, _) = player.state().expect("current");
        assert_eq!(destination, "assets/two.mp3");
    }

    #[test]
    fn a_failed_load_surfaces_as_error_state() {
        let (mut player, state) = setup();
        state.load_error.set(Some("unsupported codec".into()));
        let playback_state = player.toggle(
            "assets/bad.xyz",
            Path::new("/j/assets/bad.xyz"),
            MediaKind::Audio,
        );
        assert!(!playback_state.playing);
        assert_eq!(playback_state.error.as_deref(), Some("unsupported codec"));
        assert!(player.state().is_some(), "the failed medium stays visible");
    }

    #[test]
    fn stop_clears_the_current_medium() {
        let (mut player, _state) = setup();
        player.toggle(
            "assets/take.mp3",
            Path::new("/j/assets/take.mp3"),
            MediaKind::Audio,
        );
        player.stop();
        assert!(player.state().is_none());
    }
}
