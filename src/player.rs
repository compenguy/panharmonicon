use std::sync::mpsc;
use log::{debug, trace};

use crate::errors::{Error, Result};
use pandora_rs2;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ToPandora {
    Reset,
    Login(String, String),
    ReqStationList,
    ReqStationInfo,
    PlayStation,
    PauseStation,
    ReqSongList,
    ReqSongInfo,
    LikeSong,
    DislikeSong,
    TiredSong,
    SkipSong,
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FromPandora {
    Reset,
    AuthAccepted,
    AuthFailed,
    StationList(Vec<StationInfo>),
    StationInfo(StationInfo),
    SongList(Vec<SongInfo>),
    SongInfo(SongInfo),
    SongProgress(u8),
    Quit,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StationInfo {
    id: String,
    name: String,
}

impl std::fmt::Display for StationInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.id)
    }
}

impl From<pandora_rs2::stations::Station> for StationInfo {
    fn from(station: pandora_rs2::stations::Station) -> Self {
        Self {
            id: station.station_id,
            name: station.station_name,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SongInfo {
    is_ad: bool,
    track_token: Option<String>,
    artist_name: Option<String>,
    album_name: Option<String>,
    song_name: Option<String>,
    song_rating: Option<u32>,
}

impl std::fmt::Display for SongInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_ad {
            write!(f, "<Ad> ")?;
        }
        if let Some(name) = &self.song_name {
            write!(f, "{} ", name)?;
        }
        if let Some(artist) = &self.artist_name {
            write!(f, "by {} ", artist)?;
        }
        if let Some(album) = &self.album_name {
            write!(f, "on {} ", album)?;
        }
        if let Some(rating) = &self.song_rating {
            write!(f, "[{}]", rating)?;
        }
        Ok(())
    }
}

impl From<pandora_rs2::playlist::Track> for SongInfo {
    fn from(song: pandora_rs2::playlist::Track) -> Self {
        Self {
            is_ad: song.ad_token.is_some(),
            track_token: song.track_token,
            artist_name: song.artist_name,
            album_name: song.album_name,
            song_name: song.song_name,
            song_rating: song.song_rating,
        }
    }
}

impl From<Error> for FromPandora {
    fn from(err: Error) -> Self {
        FromPandora::Error(err.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PandoraState {
    LoggedOut,
    Authenticated,
    Playing,
    Exiting,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PandoraConfig {
    muted: bool,
    volume: u8,
}

impl Default for PandoraConfig {
    fn default() -> Self {
        PandoraConfig {
            muted: false,
            volume: u8::max_value() / 2,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Pandora {
    state: PandoraState,
    connection: Option<pandora_rs2::Pandora>,
    config: PandoraConfig,
    recv_channel: mpsc::Receiver<ToPandora>,
    send_channel: mpsc::Sender<FromPandora>,
}

impl Pandora {
    pub(crate) fn new(
        send_channel: mpsc::Sender<FromPandora>,
        recv_channel: mpsc::Receiver<ToPandora>,
        config: Option<PandoraConfig>,
    ) -> Self {
        Self {
            state: PandoraState::LoggedOut,
            connection: None,
            config: config.unwrap_or_default(),
            recv_channel,
            send_channel,
        }
    }

    pub(crate) fn try_run(&mut self) {
        loop {
            match self.recv_channel.try_recv() {
                Ok(msg) => self.process_msg(msg),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(e) => self.send_message(FromPandora::from(Error::from(e))),
            };
            trace!("Processed message from application thread");
        }
    }

    pub(crate) fn process_msg(&mut self, msg: ToPandora) {
        trace!("Action request message: {:?}", msg);
        match msg {
            ToPandora::Reset => self.reset(),
            ToPandora::Login(u, p) => self.login(&u, &p),
            ToPandora::ReqStationList => self.send_station_list(),
            ToPandora::ReqStationInfo => self.send_station_info(),
            ToPandora::PlayStation => self.play(),
            ToPandora::PauseStation => self.pause(),
            ToPandora::ReqSongList => self.send_song_list(),
            ToPandora::ReqSongInfo => self.send_song_info(),
            ToPandora::LikeSong => self.like_playing(),
            ToPandora::DislikeSong => self.dislike_playing(),
            ToPandora::TiredSong => self.tired_playing(),
            ToPandora::SkipSong => self.skip_playing(),
            ToPandora::Quit => self.quit(),
        }
    }

    pub(crate) fn send_message(&mut self, msg: FromPandora) {
        trace!("Sending message to application thread: {:?}", msg);
        if let Err(e) = self.send_channel.send(msg) {
            println!("Error communicating with main thread: {:?}", e);
            println!("Playback thread exiting...");
            self.state = PandoraState::Exiting;
        }
    }

    fn reset(&mut self) {
        trace!("Resetting player");
        self.state = PandoraState::LoggedOut;
        todo!("Actually log out of pandora")
    }

    fn login(&mut self, username: &str, password: &str) {
        trace!("Attempting Pandora login");
        match pandora_rs2::Pandora::new(username, password) {
            Ok(pandora) => {
                self.connection = Some(pandora);
                self.state = PandoraState::Authenticated;
                trace!("Pandora login successful");
            }
            Err(e) => self.send_message(FromPandora::from(Error::from(e))),
        }
    }

    fn send_station_list(&mut self) {
        trace!("Sending station list to UI");
        todo!()
    }

    fn send_station_info(&mut self) {
        trace!("Sending station info to UI");
        todo!()
    }

    fn send_song_list(&mut self) {
        trace!("Sending playlist to UI");
        todo!()
    }

    fn send_song_info(&mut self) {
        trace!("Sending song info to UI");
        todo!()
    }

    fn play(&mut self) {
        trace!("Starting playback");
        todo!()
    }

    fn pause(&mut self) {
        trace!("Pausing playback");
        todo!()
    }

    fn like_playing(&mut self) {
        trace!("Like song");
        todo!()
    }

    fn dislike_playing(&mut self) {
        trace!("Dislike song");
        todo!()
    }

    fn tired_playing(&mut self) {
        trace!("Tired of song");
        todo!()
    }

    fn skip_playing(&mut self) {
        trace!("Skip current song");
        todo!()
    }

    pub(crate) fn quit(&mut self) {
        trace!("Quit player");
        self.state = PandoraState::Exiting;
        self.send_message(FromPandora::Quit);
    }
}

pub(crate) fn run(
    send_channel: mpsc::Sender<FromPandora>,
    recv_channel: mpsc::Receiver<ToPandora>,
    config: Option<PandoraConfig>,
) {
    let mut pandora = Pandora::new(send_channel, recv_channel, config);
    while pandora.state != PandoraState::Exiting {
        trace!("Spinning on messages from ui thread");
        pandora.try_run();
        debug!("TODO: process playing song");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
