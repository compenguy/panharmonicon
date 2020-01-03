use std::sync::mpsc;

use crate::errors::{Result, Error};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ToPandora {
    Reset,
    Login,
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
    StationList,
    StationInfo,
    SongList,
    SongInfo,
    SongProgress,
    AddToPlaylist,
    Error(String),
}

impl From<Error> for FromPandora {
    fn from(err: Error) -> Self {
        FromPandora::Error(err.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PandoraState {
    LoggedOut,
    Authenticated,
    Playing,
    Exiting,
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
    config: PandoraConfig,
    recv_channel: mpsc::Receiver<ToPandora>,
    send_channel: mpsc::Sender<FromPandora>,
}

impl Pandora {
    pub(crate) fn new(send_channel: mpsc::Sender<FromPandora>, recv_channel: mpsc::Receiver<ToPandora>, config: Option<PandoraConfig>) -> Self {
        Self {
            state: PandoraState::LoggedOut,
            config: config.unwrap_or_default(),
            recv_channel,
            send_channel,
        }
    }

    pub(crate) fn send_message(&mut self, msg: FromPandora) {
        if let Err(e) = self.send_channel.send(msg) {
            println!("Error communicating with main thread: {:?}", e);
            println!("Playback thread exiting...");
            self.state = PandoraState::Exiting;
        }
    }

    pub(crate) fn process_message(&mut self) {
        match self.recv_channel.recv() {
            Ok(ToPandora::Reset) => self.reset(),
            Ok(ToPandora::Login) => self.login(),
            Ok(ToPandora::ReqStationList) => self.send_station_list(),
            Ok(ToPandora::ReqStationInfo) => self.send_station_info(),
            Ok(ToPandora::PlayStation) => self.play(),
            Ok(ToPandora::PauseStation) => self.pause(),
            Ok(ToPandora::ReqSongList) => self.send_song_list(),
            Ok(ToPandora::ReqSongInfo) => self.send_song_info(),
            Ok(ToPandora::LikeSong) => self.like_playing(),
            Ok(ToPandora::DislikeSong) => self.dislike_playing(),
            Ok(ToPandora::TiredSong) => self.tired_playing(),
            Ok(ToPandora::SkipSong) => self.skip_playing(),
            Ok(ToPandora::Quit) => self.quit(),
            Err(e) => self.send_message(FromPandora::from(Error::from(e))),
        }
    }

    fn reset(&mut self) {
        todo!()
    }

    fn login(&mut self) {
        todo!()
    }

    fn send_station_list(&mut self) {
        todo!()
    }

    fn send_station_info(&mut self) {
        todo!()
    }

    fn send_song_list(&mut self) {
        todo!()
    }

    fn send_song_info(&mut self) {
        todo!()
    }

    fn play(&mut self) {
        todo!()
    }

    fn pause(&mut self) {
        todo!()
    }

    fn like_playing(&mut self) {
        todo!()
    }

    fn dislike_playing(&mut self) {
        todo!()
    }

    fn tired_playing(&mut self) {
        todo!()
    }

    fn skip_playing(&mut self) {
        todo!()
    }

    pub(crate) fn quit(&mut self) {
        todo!()
    }
}

pub(crate) fn run(send_channel: mpsc::Sender<FromPandora>, recv_channel: mpsc::Receiver<ToPandora>, config: Option<PandoraConfig>) {
    let mut pandora = Pandora::new(send_channel, recv_channel, config);
    while pandora.state != PandoraState::Exiting {
        pandora.process_message();
    }
}
