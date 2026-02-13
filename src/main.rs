use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::messages::State;

use anyhow::{Context, Result};
use flexi_logger::{detailed_format, Logger};
use log::{debug, error, trace};

mod errors;
use crate::errors::Error;

mod config;
use crate::config::{Config, SharedConfig};

mod caching;
mod messages;
mod model;
#[cfg(feature = "mpris_server")]
mod mpris_ui;
mod pandora;
mod player;
#[cfg(feature = "term_ui")]
mod term_ui;
mod track;

fn main() -> Result<()> {
    human_panic::setup_panic!();

    let config_file = dirs::config_dir()
        .ok_or(Error::AppDirNotFound)?
        .join(clap::crate_name!())
        .join("config.json");
    let mut app = clap::command!("")
        .arg(
            clap::Arg::new("gen-config")
                .short('c')
                .long("gen-config")
                .action(clap::ArgAction::SetTrue)
                .help(format!(
                    "Generate a default config file at {}",
                    config_file.to_string_lossy()
                )),
        )
        .arg(
            clap::Arg::new("debug")
                .short('g')
                .long("debug")
                .action(clap::ArgAction::Count)
                .hide(true)
                .help("Enable debug-level output"),
        )
        .arg(
            clap::Arg::new("debug-log")
                .short('l')
                .long("debug-log")
                .action(clap::ArgAction::SetTrue)
                .hide(true)
                .help("Whether to write a debug log file."),
        );
    #[cfg(feature = "term_ui")]
    {
        app = app.arg(
            clap::Arg::new("terminal")
                .short('t')
                .long("terminal")
                .action(clap::ArgAction::SetTrue)
                .help("Run with the terminal UI (only applies when MPRIS is also available)."),
        );
    }
    let matches = app.get_matches();

    let crate_log_level = match matches.get_count("debug") {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };
    let general_log_level = match crate_log_level {
        log::LevelFilter::Trace | log::LevelFilter::Debug => log::LevelFilter::Error,
        _ => log::LevelFilter::Off,
    };
    let spec = format!(
        "{}, {} = {}",
        general_log_level,
        clap::crate_name!(),
        crate_log_level
    );
    let mut log_builder = Logger::try_with_str(&spec)?.format(detailed_format);

    if matches
        .get_one::<bool>("debug-log")
        .copied()
        .unwrap_or(false)
    {
        let data_local_dir = dirs::data_local_dir()
            .ok_or(Error::AppDirNotFound)?
            .join(clap::crate_name!());
        let log_dir = data_local_dir
            .join("logs")
            .join(format!("{}", chrono::offset::Utc::now()));
        if !log_dir.is_dir() {
            std::fs::create_dir_all(&log_dir).with_context(|| {
                format!(
                    "Failed to create application directory {}",
                    log_dir.to_string_lossy()
                )
            })?;
        }
        log_builder = log_builder.log_to_file(
            flexi_logger::FileSpec::default()
                .suppress_timestamp()
                .directory(&log_dir),
        );
        println!("Logging debug output to {}", log_dir.to_string_lossy());
    }

    log_builder
        .start()
        .context("Failed to start FlexiLogger logging backend")?;

    debug!("{} version {}", clap::crate_name!(), clap::crate_version!());

    trace!("Loading user config");
    let gen_config = matches
        .get_one::<bool>("gen-config")
        .copied()
        .unwrap_or(false);
    let conf = Config::get_config(config_file, gen_config)?;
    debug!("Configuration settings: {:?}", &conf);
    let shared_config: SharedConfig = Arc::new(RwLock::new(conf));

    trace!("Initializing Pandora API task");
    let (pandora_cmd_tx, pandora_cmd_rx) = tokio::sync::mpsc::channel(32);
    let (pandora_result_tx, pandora_result_rx) = tokio::sync::mpsc::channel(32);

    trace!("Initializing application core");
    let model = model::Model::new(shared_config.clone(), pandora_cmd_tx, pandora_result_rx);

    trace!("Initializing track fetcher");
    let mut fetcher = caching::TrackCacher::new(model.updates_channel(), model.request_channel());

    #[cfg(all(feature = "term_ui", feature = "mpris_server"))]
    let use_terminal_ui = matches.get_flag("terminal");
    #[cfg(all(feature = "term_ui", not(feature = "mpris_server")))]
    let use_terminal_ui = true;

    trace!("Initializing player interface");
    let mut player = player::Player::new(model.updates_channel(), model.request_channel());

    // Polling interval for main loop and worker tasks. ~50ms keeps UI/control latency low without extra CPU.
    let naptime = Duration::from_millis(50);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("Failed to build tokio runtime")?;

    rt.block_on(async move {
        let pandora_handle = tokio::spawn(pandora::run_pandora_task(
            shared_config.clone(),
            pandora_cmd_rx,
            pandora_result_tx,
        ));

        let mut state_receiver_main = model.updates_channel();
        let mut state_receiver_fetcher = model.updates_channel();
        #[cfg(feature = "mpris_server")]
        let mut state_receiver_mpris = model.updates_channel();

        #[cfg(feature = "mpris_server")]
        let mut mpris_ui = mpris_ui::MprisUi::new(model.updates_channel(), model.request_channel())
            .await
            .expect("Failed to create MPRIS UI");

        let mut term_ui_opt = if use_terminal_ui && cfg!(feature = "term_ui") {
            Some(term_ui::Terminal::new(
                shared_config.clone(),
                model.updates_channel(),
                model.request_channel(),
            ))
        } else {
            None
        };

        // Model: event-driven; wake on request or on timer; exits when it processes Request::Quit.
        let model_handle = tokio::spawn(async move {
            let mut model = model;
            if let Err(e) = model.run_until_quit(naptime).await {
                error!("Error in model: {e:#}");
            }
            drop(model);
        });

        // Fetcher: event-driven; wake on state (Quit) or timer for update.
        let fetcher_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    result = state_receiver_fetcher.recv() => {
                        if let Ok(msg) = result {
                            if matches!(msg, State::Quit) {
                                return;
                            }
                        }
                        while let Ok(msg) = state_receiver_fetcher.try_recv() {
                            if matches!(msg, State::Quit) {
                                return;
                            }
                        }
                    }
                    _ = tokio::time::sleep(naptime) => {
                        if let Err(e) = fetcher.update().await {
                            error!("Error updating fetcher state: {e:#}");
                        }
                    }
                }
            }
        });

        #[cfg(feature = "mpris_server")]
        let mpris_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    result = state_receiver_mpris.recv() => {
                        if let Ok(msg) = result {
                            if matches!(msg, State::Quit) {
                                return;
                            }
                        }
                        while let Ok(msg) = state_receiver_mpris.try_recv() {
                            if matches!(msg, State::Quit) {
                                return;
                            }
                        }
                    }
                    _ = tokio::time::sleep(naptime) => {
                        if let Err(e) = mpris_ui.update().await {
                            error!("Error updating mpris state: {e:#}");
                        }
                    }
                }
            }
        });

        // Player and term_ui (cursive) stay on the main thread. Poll every naptime so the UI
        // always gets to run and process input; drain state first to see Quit promptly.
        trace!("Starting app (player and term_ui on main thread)");
        let mut quitting = false;
        while !quitting {
            while let Ok(msg) = state_receiver_main.try_recv() {
                if matches!(msg, State::Quit) {
                    quitting = true;
                    continue;
                }
            }
            if let Some(term_ui) = term_ui_opt.as_mut() {
                let _ = tokio::try_join!(player.update(), term_ui.update())
                    .map_err(|e| error!("Error updating application state: {e:#}"));
            } else if let Err(e) = player.update().await {
                error!("Error updating player state: {e:#}");
            }
            tokio::time::sleep(naptime).await;
        }
        drop(player);

        let _ = model_handle.await;
        let _ = fetcher_handle.await;
        #[cfg(feature = "mpris_server")]
        let _ = mpris_handle.await;
        let _ = pandora_handle.await;
    });

    debug!("Application quit request acknowledged.");
    debug!("Application interface terminated.");

    Ok(())
}
