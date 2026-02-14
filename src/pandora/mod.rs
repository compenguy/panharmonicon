//! Dedicated subsystem and task for Pandora API requests.
//! The model sends commands and receives results asynchronously so it can keep processing
//! UI/player requests while network I/O runs here.

use std::collections::HashMap;
use std::convert::TryFrom;

use anyhow::Result;
use log::{debug, error, trace, warn};
use tokio::sync::mpsc;

use crate::config::SharedConfig;
use crate::errors::Error;
use crate::pandora::api::PandoraSession;
use crate::track::Track;

mod api;

/// Commands the model sends to the Pandora task.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum PandoraCommand {
    Connect,
    Disconnect,
    GetStationList,
    GetPlaylist(String),
    RateTrack(Track, Option<bool>),
    /// Add an artist or track as a seed for the given station (music_token from search).
    AddSeed {
        station_id: String,
        music_token: String,
    },
    /// List seeds (artists, tracks, genres) for a station.
    ListSeeds(String),
    /// List rated tracks (loved/thumbs up, banned/thumbs down) for a station.
    ListRatedTracks(String),
    /// Create a station from a track token (e.g. from playlist); as_artist = use artist as seed.
    CreateStationFromTrack {
        track_token: String,
        as_artist: bool,
    },
    /// Create a station from a music token (e.g. from search).
    CreateStationFromMusic {
        music_token: String,
    },
    /// Delete a station by its station token.
    DeleteStation(String),
    /// Add the current artist as a station seed (search by name, then add first match).
    AddArtistSeed {
        station_id: String,
        artist_name: String,
    },
    /// Remove a seed by its seed_id (from station seeds).
    RemoveSeed(String),
    Quit,
}

/// Seed info for one artist or song seed on a station.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ArtistSeedInfo {
    pub seed_id: String,
    pub music_token: String,
    pub artist_name: String,
}

/// Seed info for one song seed on a station.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct SongSeedInfo {
    pub seed_id: String,
    pub music_token: String,
    pub song_name: String,
    pub artist_name: String,
}

/// All seeds for a station.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct StationSeedsData {
    pub artist_seeds: Vec<ArtistSeedInfo>,
    pub song_seeds: Vec<SongSeedInfo>,
}

/// One rated track (loved or banned) on a station.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct RatedTrackInfo {
    pub feedback_id: String,
    pub music_token: String,
    pub song_name: String,
    pub artist_name: String,
    pub is_positive: bool,
}

/// All rated tracks (thumbs up / thumbs down) for a station.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct StationRatedTracksData {
    pub thumbs_up: Vec<RatedTrackInfo>,
    pub thumbs_down: Vec<RatedTrackInfo>,
}

/// Results the Pandora task sends back to the model.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum PandoraResult {
    Connected,
    AuthFailed(String),
    Disconnected,
    StationList(HashMap<String, String>),
    Playlist(Vec<Track>),
    Rated(u32),
    Seeds(String, StationSeedsData),
    RatedTracks(StationRatedTracksData),
    StationCreated {
        station_token: String,
        station_name: String,
    },
    StationDeleted,
    SeedAdded {
        seed_id: String,
        artist_name: String,
    },
    SeedRemoved,
    Error(String),
    QuitAck,
}

/// Runs the Pandora API task. Receives commands, performs session/network work, sends results.
pub(crate) async fn run_pandora_task(
    config: SharedConfig,
    mut command_rx: mpsc::Receiver<PandoraCommand>,
    result_tx: mpsc::Sender<PandoraResult>,
) {
    let mut session: Option<PandoraSession> = None;

    while let Some(cmd) = command_rx.recv().await {
        match cmd {
            PandoraCommand::Connect => {
                if session.as_ref().is_some_and(|s| s.connected()) {
                    debug!("Pandora task: already connected, ignoring Connect");
                    continue;
                }
                let mut new_session = PandoraSession::new(config.clone());
                match do_connect(&mut new_session).await {
                    Ok(()) => {
                        session = Some(new_session);
                        trace!("Pandora task: connected");
                        let _ = result_tx.send(PandoraResult::Connected).await;
                    }
                    Err(e) => {
                        let message = if e
                            .downcast_ref::<Error>()
                            .is_some_and(|e| e.missing_auth_token())
                        {
                            String::from("Required authentication token is missing.")
                        } else if let Some(e) = e.downcast_ref::<pandora_api::errors::Error>() {
                            format!("Pandora authentication failure: {e:#}")
                        } else {
                            format!("Unknown error while logging in: {e:#}")
                        };
                        error!("Pandora task: connect failed: {message}");
                        session = None;
                        let _ = result_tx.send(PandoraResult::AuthFailed(message)).await;
                    }
                }
            }
            PandoraCommand::Disconnect => {
                if let Some(mut s) = session.take() {
                    trace!("Pandora task: disconnecting...");
                    s.partner_logout().await;
                    trace!("Pandora task: disconnected");
                }
                let _ = result_tx.send(PandoraResult::Disconnected).await;
            }
            PandoraCommand::GetStationList => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: GetStationList while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                match sess.get_station_list().await {
                    Ok(resp) => {
                        let list = resp
                            .stations
                            .into_iter()
                            .map(|s| (s.station_id, s.station_name))
                            .collect::<HashMap<_, _>>();
                        let _ = result_tx.send(PandoraResult::StationList(list)).await;
                    }
                    Err(e) => {
                        error!("Pandora task: get_station_list failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::GetPlaylist(station_id) => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: GetPlaylist while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                match sess.get_playlist(&station_id).await {
                    Ok(entries) => {
                        let playlist: Result<Vec<Track>> = entries
                            .into_iter()
                            .flat_map(|pe| pe.get_track().map(Track::try_from).into_iter())
                            .collect();
                        match playlist {
                            Ok(tracks) => {
                                debug!("Pandora task: got {} tracks", tracks.len());
                                let _ = result_tx.send(PandoraResult::Playlist(tracks)).await;
                            }
                            Err(e) => {
                                error!("Pandora task: playlist track conversion failed: {e:#}");
                                let _ =
                                    result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Pandora task: get_playlist failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::RateTrack(track, rating) => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: RateTrack while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                let new_rating_value = if rating.unwrap_or(false) { 1 } else { 0 };
                let res = if let Some(r) = rating {
                    sess.add_feedback(&track, r).await.map(|_| ())
                } else {
                    sess.delete_feedback_for_track(&track).await
                };
                match res {
                    Ok(()) => {
                        trace!("Pandora task: rated track");
                        let _ = result_tx.send(PandoraResult::Rated(new_rating_value)).await;
                    }
                    Err(e) => {
                        error!("Pandora task: rate failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::AddSeed {
                station_id,
                music_token,
            } => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: AddSeed while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                match sess.add_music(&station_id, &music_token).await {
                    Ok(resp) => {
                        trace!("Pandora task: added seed");
                        let _ = result_tx
                            .send(PandoraResult::SeedAdded {
                                seed_id: resp.seed_id,
                                artist_name: resp.artist_name,
                            })
                            .await;
                    }
                    Err(e) => {
                        error!("Pandora task: add_music failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::ListSeeds(station_id) => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: ListSeeds while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                let station_id = station_id.clone();
                match sess.get_station_seeds(&station_id).await {
                    Ok(seeds) => {
                        let _ = result_tx.send(PandoraResult::Seeds(station_id, seeds)).await;
                    }
                    Err(e) => {
                        error!("Pandora task: get_station_seeds failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::ListRatedTracks(station_id) => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: ListRatedTracks while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                match sess.get_station_rated_tracks(&station_id).await {
                    Ok(rated) => {
                        let _ = result_tx.send(PandoraResult::RatedTracks(rated)).await;
                    }
                    Err(e) => {
                        error!("Pandora task: get_station_rated_tracks failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::CreateStationFromTrack {
                track_token,
                as_artist,
            } => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: CreateStationFromTrack while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                let create = if as_artist {
                    sess.create_station_from_track_artist(&track_token).await
                } else {
                    sess.create_station_from_track_song(&track_token).await
                };
                match create {
                    Ok(resp) => {
                        trace!("Pandora task: created station from track");
                        let _ = result_tx
                            .send(PandoraResult::StationCreated {
                                station_token: resp.station_token.clone(),
                                station_name: String::new(),
                            })
                            .await;
                    }
                    Err(e) => {
                        error!("Pandora task: create_station failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::CreateStationFromMusic { music_token } => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: CreateStationFromMusic while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                match sess.create_station_from_music_token(&music_token).await {
                    Ok(resp) => {
                        trace!("Pandora task: created station from music");
                        let _ = result_tx
                            .send(PandoraResult::StationCreated {
                                station_token: resp.station_token.clone(),
                                station_name: String::new(),
                            })
                            .await;
                    }
                    Err(e) => {
                        error!("Pandora task: create_station failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::DeleteStation(station_id) => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: DeleteStation while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                match sess.delete_station(&station_id).await {
                    Ok(()) => {
                        trace!("Pandora task: deleted station");
                        let _ = result_tx.send(PandoraResult::StationDeleted).await;
                    }
                    Err(e) => {
                        error!("Pandora task: delete_station failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::AddArtistSeed {
                station_id,
                artist_name,
            } => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: AddArtistSeed while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                match sess.search(&artist_name).await {
                    Ok(resp) => {
                        if let Some(artist) = resp.artists.first() {
                            match sess.add_music(&station_id, &artist.music_token).await {
                                Ok(resp) => {
                                    trace!("Pandora task: added artist seed");
                                    let _ = result_tx
                                        .send(PandoraResult::SeedAdded {
                                            seed_id: resp.seed_id,
                                            artist_name: resp.artist_name,
                                        })
                                        .await;
                                }
                                Err(e) => {
                                    error!("Pandora task: add_music (artist) failed: {e:#}");
                                    let _ = result_tx
                                        .send(PandoraResult::Error(format!("{e:#}")))
                                        .await;
                                }
                            }
                        } else {
                            warn!("Pandora task: no artist match for '{artist_name}'");
                            let _ = result_tx
                                .send(PandoraResult::Error(format!(
                                    "No artist found for '{artist_name}'"
                                )))
                                .await;
                        }
                    }
                    Err(e) => {
                        error!("Pandora task: search for artist failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::RemoveSeed(seed_id) => {
                let sess = match session.as_mut() {
                    Some(s) if s.connected() => s,
                    _ => {
                        warn!("Pandora task: RemoveSeed while not connected");
                        let _ = result_tx
                            .send(PandoraResult::Error("Not connected".into()))
                            .await;
                        continue;
                    }
                };
                match sess.delete_music(&seed_id).await {
                    Ok(()) => {
                        trace!("Pandora task: removed seed");
                        let _ = result_tx.send(PandoraResult::SeedRemoved).await;
                    }
                    Err(e) => {
                        error!("Pandora task: delete_music failed: {e:#}");
                        let _ = result_tx.send(PandoraResult::Error(format!("{e:#}"))).await;
                    }
                }
            }
            PandoraCommand::Quit => {
                if let Some(mut s) = session.take() {
                    s.partner_logout().await;
                }
                let _ = result_tx.send(PandoraResult::QuitAck).await;
                return;
            }
        }
    }
}

async fn do_connect(session: &mut PandoraSession) -> Result<()> {
    trace!("Connecting to Pandora...");
    session.partner_login().await?;
    session.user_login().await?;
    if session.connected() {
        trace!("Connected to Pandora.");
    } else {
        error!("Pandora session reports not connected after login");
    }
    Ok(())
}
