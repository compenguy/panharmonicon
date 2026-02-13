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
pub(crate) enum PandoraCommand {
    Connect,
    Disconnect,
    GetStationList,
    GetPlaylist(String),
    RateTrack(Track, Option<bool>),
    Quit,
}

/// Results the Pandora task sends back to the model.
#[derive(Debug)]
pub(crate) enum PandoraResult {
    Connected,
    AuthFailed(String),
    Disconnected,
    StationList(HashMap<String, String>),
    Playlist(Vec<Track>),
    Rated(u32),
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
