use std::{cell::RefCell, rc::Rc};

use anyhow::{Context, Result};
use flexi_logger::{detailed_format, Logger};
use log::{debug, error, trace};

mod errors;
use crate::errors::Error;

mod config;
use crate::config::Config;

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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
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
    let conf_ref = Rc::new(RefCell::new(conf));

    trace!("Initializing application core");
    let mut model = model::Model::new(conf_ref.clone());

    trace!("Initializing track fetcher");
    let mut fetcher = caching::TrackCacher::new(model.updates_channel(), model.request_channel());

    #[cfg(feature = "mpris_server")]
    trace!("Initializing mpris interface");
    #[cfg(feature = "mpris_server")]
    let mut mpris_ui =
        mpris_ui::MprisUi::new(model.updates_channel(), model.request_channel()).await?;

    // When both term_ui and mpris_server are enabled, only run the terminal UI if --terminal was passed.
    #[cfg(all(feature = "term_ui", feature = "mpris_server"))]
    let use_terminal_ui = matches.get_flag("terminal");
    #[cfg(all(feature = "term_ui", not(feature = "mpris_server")))]
    let use_terminal_ui = true;

    #[cfg(all(feature = "term_ui", not(feature = "mpris_server")))]
    trace!("Initializing terminal interface");
    #[cfg(all(feature = "term_ui", not(feature = "mpris_server")))]
    let mut term_ui =
        term_ui::Terminal::new(conf_ref, model.updates_channel(), model.request_channel());

    #[cfg(all(feature = "term_ui", feature = "mpris_server"))]
    let mut term_ui_opt = if use_terminal_ui {
        trace!("Initializing terminal interface");
        Some(term_ui::Terminal::new(
            conf_ref,
            model.updates_channel(),
            model.request_channel(),
        ))
    } else {
        let _ = conf_ref;
        None
    };

    trace!("Initializing player interface");
    let mut player = player::Player::new(model.updates_channel(), model.request_channel());

    trace!("Starting app");
    let naptime = std::time::Duration::from_millis(100);

    // TODO: spawn each subsystem, or at least the `Send` ones, as their own task?
    while !model.quitting() {
        trace!("Advancing application state...");
        /*
        let mut dirty = false;
        trace!("Advancing model...");
        match model.update().await {
            Ok(d) => dirty |= d,
            Err(e) => error!("Error updating model state: {e:#}"),
        }

        trace!("Advancing player...");
        match player.update().await {
            Ok(d) => dirty |= d,
            Err(e) => error!("Error updating player state: {e:#}"),
        }

        trace!("Advancing ui...");
        match ui.update().await {
            Ok(d) => dirty |= d,
            Err(e) => error!("Error updating ui state: {e:#}"),
        }

        trace!("Advancing fetcher...");
        match fetcher.update().await {
            Ok(d) => dirty |= d,
            Err(e) => error!("Error updating fetcher state: {e:#}"),
        }

        if !dirty {
            trace!("naptime");
            std::thread::sleep(naptime);
        }
        */

        #[cfg(all(not(feature = "mpris_server"), not(feature = "term_ui")))]
        {
            let step_result = tokio::try_join!(model.update(), player.update(), fetcher.update());
            match step_result {
                Err(e) => error!("Error updating application state: {e:#}"),
                Ok((false, false, false)) => std::thread::sleep(naptime),
                Ok((_, _, _)) => (),
            }
        }
        #[cfg(all(not(feature = "mpris_server"), feature = "term_ui"))]
        {
            let step_result = tokio::try_join!(
                model.update(),
                player.update(),
                term_ui.update(),
                fetcher.update()
            );
            match step_result {
                Err(e) => error!("Error updating application state: {e:#}"),
                Ok((false, false, false, false)) => std::thread::sleep(naptime),
                Ok((_, _, _, _)) => (),
            }
        }
        #[cfg(all(feature = "mpris_server", not(feature = "term_ui")))]
        {
            let step_result = tokio::try_join!(
                model.update(),
                player.update(),
                mpris_ui.update(),
                fetcher.update()
            );
            match step_result {
                Err(e) => error!("Error updating application state: {e:#}"),
                Ok((false, false, false, false)) => std::thread::sleep(naptime),
                Ok((_, _, _, _)) => (),
            }
        }
        #[cfg(all(feature = "mpris_server", feature = "term_ui"))]
        {
            if use_terminal_ui {
                let step_result = tokio::try_join!(
                    model.update(),
                    player.update(),
                    term_ui_opt.as_mut().unwrap().update(),
                    mpris_ui.update(),
                    fetcher.update()
                );
                match step_result {
                    Err(e) => error!("Error updating application state: {e:#}"),
                    Ok((false, false, false, false, false)) => std::thread::sleep(naptime),
                    Ok((_, _, _, _, _)) => (),
                }
            } else {
                let step_result = tokio::try_join!(
                    model.update(),
                    player.update(),
                    mpris_ui.update(),
                    fetcher.update()
                );
                match step_result {
                    Err(e) => error!("Error updating application state: {e:#}"),
                    Ok((false, false, false, false)) => std::thread::sleep(naptime),
                    Ok((_, _, _, _)) => (),
                }
            }
        }
    }
    debug!("Application quit request acknowledged.");
    // Explicitly drop the model to force it to write changed settings out
    drop(model);
    debug!("Application interface terminated.");

    Ok(())
}
