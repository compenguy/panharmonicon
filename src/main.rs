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
mod pandora;
mod term_ui;
mod track;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let config_file = dirs::config_dir()
        .ok_or(Error::AppDirNotFound)?
        .join(clap::crate_name!())
        .join("config.json");
    let matches = clap::command!("")
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
        )
        .get_matches();

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
    let req_chan = model.init_request_channel();
    let notif_chan = model.init_notification_channel();

    trace!("Initializing track fetcher");
    let mut fetcher = caching::TrackCacher::new(notif_chan.clone(), req_chan.clone());

    trace!("Initializing terminal interface");
    let mut ui = term_ui::Terminal::new(conf_ref, notif_chan, req_chan);

    trace!("Starting app");
    let naptime = std::time::Duration::from_millis(100);
    while !model.quitting() {
        trace!("Advancing application state...");
        let step_result = tokio::try_join!(model.update(), ui.update(), fetcher.update());
        match step_result {
            Err(e) => error!("Error updating application state: {:?}", e),
            Ok((false, false, false)) => std::thread::sleep(naptime),
            Ok((_, _, _)) => (),
        }
    }
    debug!("Application quit request acknowledged.");
    // Explicitly drop the UI to force it to write changed settings out
    drop(ui);
    debug!("Application interface terminated.");

    Ok(())
}
