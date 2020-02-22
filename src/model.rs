use crate::config::Config::Credentials;
use pandora_api::json::user::Station;

pub(crate) trait StateMediator {
    fn disconnected(&self) -> bool;
    fn disconnect(&mut self);
    fn fail_authentication(&mut self);
    fn connected(&self) -> bool;
    fn connect(&mut self);
    fn tuned(&self) -> Option<String>;
    fn tune(&mut self, station_id: String);
    fn ready(&self) -> bool;
    fn playing(&self) -> Option<PlaylistTrack>;
    fn start(&mut self);
}

pub(crate) trait PlaybackMediator {
    fn stopped(&self) -> bool;
    fn stop(&mut self);
    fn paused(&self) -> bool;
    fn pause(&mut self);
    fn unpause(&mut self);
    fn toggle_pause(&mut self) {
        if self.paused() {
            self.unpause();
        } else {
            self.pause();
        }
    }
    fn volume(&self) -> f32;
    fn set_volume(&mut self, new_volume: f32);
    fn refresh_volume(&mut self);
    fn muted(&self) -> bool;
    fn mute(&mut self);
    fn unmute(&mut self);
    fn toggle_mute(&mut self) {
        if self.muted() {
            self.unmute();
        } else {
            self.mute();
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum CachePolicy {
    NoCaching,
    CachePlayingEvictCompleted,
    CacheNextEvictCompleted,
    CacheAllNoEviction,
}

impl CachePolicy {
    pub(crate) fn cache_playing(&self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => true,
            Self::CacheNextEvictCompleted => true,
            Self::CacheAllNoEviction => true,
        }
    }

    pub(crate) fn cache_plus_one(&self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => false,
            Self::CacheNextEvictCompleted => true,
            Self::CacheAllNoEviction => true,
        }
    }

    pub(crate) fn cache_all(&self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => false,
            Self::CacheNextEvictCompleted => false,
            Self::CacheAllNoEviction => true,
        }
    }

    pub(crate) fn evict_completed(&self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => true,
            Self::CacheNextEvictCompleted => true,
            Self::CacheAllNoEviction => false,
        }
    }
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self::CachePlayingEvictCompleted
    }
}

#[derive(Debug, Clone, Copy)]
enum Volume {
    Muted(f32),
    Unmuted(f32),
}

impl Volume {
    fn volume(&self) -> f32 {
        if let Self::Unmuted(v) = self {
            v.min(0.0f32).max(1.0f32)
        } else {
            0.0f32
        }
    }

    fn set_volume(&self, new_volume: f32) {
        *self = Self::Unmuted(new_volume.min(0.0f32).max(1.0f32));
    }

    fn increase_volume(&mut self) {
        self.set_volume(self.volume() + 0.1);
    }

    fn decrease_volume(&mut self) {
        self.set_volume(self.volume() - 0.1);
    }

    fn muted(&self) -> bool {
        match self {
            Self::Muted(_) => true,
            Self::Unmuted(_) => false,
        }
    }

    fn mute(&mut self) {
        let volume = self.volume();
        *self = Self::Muted(volume);
    }

    fn unmute(&mut self) {
        let volume = self.volume();
        *self = Self::Unmuted(volume);
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self::Unmuted(1.0f32)
    }
}

#[derive(Debug, Clone)]
struct AudioDevice {
    device: rodio::Device,
    sink: rodio::Sink,
    volume: Volume,
}

impl AudioDevice {
    fn play_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        self.play_from_reader(std::io::BufReader::new(std::fs::File::new(path)?))
    }

    fn play_from_reader<R: Read + Seek + Send + 'static>(&mut self, reader: R) -> Result<()> {
        let start_paused = false;
        let decoder = rodio::decoder::Decoder::new(reader)?.pausable(start_paused);

        // Force the sink to be deleted and recreated, ensuring it's in
        // a good state
        self.stop();

        self.audio_sink.append(decoder);
    }
}

impl PlaybackMediator for AudioDevice {
    fn stopped(&self) -> bool {
        self.sink.empty()
    }

    fn stop(&mut self) {
        self.sink = rodio::Sink::new(&self.device);
        self.sink.set_volume(self.volume.volume());
    }

    fn paused(&self) -> bool {
        self.sink.is_paused()
    }

    fn pause(&mut self) {
        self.sink.pause();
    }

    fn unpause(&mut self) {
        self.sink.play()
    }

    fn volume(&self) -> f32 {
        self.volume.volume()
    }

    fn set_volume(&mut self, new_volume: f32) {
        self.volume.set_volume(new_volume);
        self.refresh_volume();
    }

    fn refresh_volume(&mut self) {
        self.sink.set_volume(self.volume.volume());
    }

    fn muted(&self) -> bool {
        self.volume.muted()
    }

    fn mute(&mut self) {
        self.volume.mute();
        self.refresh_volume();
    }

    fn unmute(&mut self) {
        self.volume.unmute();
        self.refresh_volume();
    }
}

impl Default for AudioDevice {
    fn default() -> Self {
        let device = rodio::default_audio_device().expect("Failed to locate/initialize default audio device");
        let sink = rodio::Sink::new(&device);
        Self {
            device,
            sink,
            volume: Volume::default()
        }
    }
}

#[derive(Debug, Clone)]
struct Playing {
    audio_device: AudioDevice,
    last_started: Option<Instant>,
    elapsed: Duration,
    duration: Duration,
    playlist: VecDeque<PlaylistTrack>,
}

impl PlaybackMediator for Playing {
    fn stopped(&self) -> bool {
        self.audio_device.empty()
    }

    fn stop(&mut self) {
        if self.last_started.is_some() {
            self.audio_device.stop();
            self.playlist.pop_front();
            self.last_started = None;
            self.elapsed = Duration::default();
            self.duration = Duration::default();
        }
    }

    fn paused(&self) -> bool {
        assert_eq!(self.last_started.is_none(), self.audio_device.paused());
        self.last_started.is_none()
    }

    fn pause(&mut self) {
        self.elapsed += self.last_started.take().unwrap_or_default();
        self.audio_device.pause();
    }

    fn unpause(&mut self) {
        if self.last_started.is_none() {
            self.last_started = Instant::now();
            self.audio_device.play();
        }
    }

    fn volume(&self) -> f32 {
        self.audio_device.volume()
    }

    fn set_volume(&mut self, new_volume: f32) {
        self.audio_device.set_volume()
    }

    fn refresh_volume(&mut self) {
        self.audio_device.refresh_volume();
    }

    fn muted(&self) -> bool {
        self.audio_device.muted()
    }

    fn mute(&mut self) {
        self.audio_device.mute();
    }

    fn unmute(&mut self) {
        self.audio_device.unmute();
    }
}

impl Default for Playing {
    fn default() -> Self {
        Self {
            audio_device: AudioDevice::default(),
            last_started: None,
            elapsed: Duration::default(),
            duration: Duration::default(),
            playlist: VecDeque::default(),
        }
    }
}

#[derive(Debug, Clone)]
struct Model {
    config: Rc<RefCell<Config>>,
    session: PandoraSession,
    credentials: Option<Credentials>,
    station: Option<String>,
    station_list: HashMap<String, Station>,
    playing: Playing,
}

impl Model {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        Self {
            config: config.clone(),
            session: PandoraSession::new(config),
            // TODO: initialize this from config
            credentials: None,
            // TODO: initialize this from config
            station: None,
            station_list: HashMap::new(),
            playing: Playing::default(),
        }
    }
}

impl PlaybackMediator for Model {
    fn stopped(&self) -> bool {
        self.playing.empty()
    }

    fn stop(&mut self) {
        self.playing.stop();
    }

    fn paused(&self) -> bool {
        self.playing.paused()
    }

    fn pause(&mut self) {
        self.playing.pause();
    }

    fn unpause(&mut self) {
        self.playing.unpause();
    }

    fn volume(&self) -> f32 {
        self.playing.volume()
    }

    fn set_volume(&mut self, new_volume: f32) {
        self.playing.set_volume()
    }

    fn refresh_volume(&mut self) {
        self.playing.refresh_volume();
    }

    fn muted(&self) -> bool {
        self.playing.muted()
    }

    fn mute(&mut self) {
        self.playing.mute();
    }

    fn unmute(&mut self) {
        self.playing.unmute();
    }
}

impl StateMediator for Model {
    config: Rc<RefCell<Config>>,
    session: PandoraSession,
    credentials: Option<Credentials>,
    station: Option<String>,
    station_list: HashMap<String, Station>,
    playing: Playing,
    fn disconnected(&self) -> bool {
        !self.session.connected()
    }

    fn disconnect(&mut self) {
        // TODO: Evaluate whether session.user_logout() would better suit
        self.session.partner_logout();
    }

    fn fail_authentication(&mut self) {
        self.credentials = None;
    }

    fn connected(&self) -> bool {
        self.session.connected();
    }

    fn connect(&mut self) {
        if let Err(e) = lself.session.user_login() {
            error!("Login error: {}", e);
        }
    }

    fn tuned(&self) -> Option<String> {
        if self.connected() {
            self.station.clone()
        } else {
            None
        }
    }

    fn tune(&mut self, station_id: String) {
        self.station = Some(station_id);
    }

    fn untune(&mut self) {
        self.station = None;
    }

    fn ready(&self) -> bool {
        self.stopped()
    }

    fn playing(&self) -> Option<PlaylistTrack> {
        self.playing.playing()
    }

    fn start(&mut self) {
        todo!("Get playlist, add tracks to playing's playlist, cache first entry, then start it playing")
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        // TODO: commit self.credentials and self.station to config
        // and flush to disk
    }
}

