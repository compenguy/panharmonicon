//#![feature(with_options)]
use std::{cell::RefCell, rc::Rc};

use anyhow::{Context, Result};
use clap::{app_from_crate, crate_name, crate_version};
use flexi_logger::{colored_default_format, detailed_format, Logger};
use human_panic::setup_panic;
use log::{debug, trace};

mod errors;
use crate::errors::Error;

mod config;
use crate::config::Config;

mod caching;
mod model;
mod pandora;
mod term_ui;

fn main() -> Result<()> {
    setup_panic!(Metadata {
        name: env!("CARGO_PKG_NAME").into(),
        version: env!("CARGO_PKG_VERSION").into(),
        authors: "Will Page <compenguy@gmail.com>".into(),
        homepage: "".into(),
    });
    let config_file = dirs::config_dir()
        .ok_or(Error::AppDirNotFound)?
        .join(crate_name!())
        .join("config.json");
    let matches = app_from_crate!("")
        .setting(clap::AppSettings::ColorAuto)
        .setting(clap::AppSettings::ColoredHelp)
        .arg(
            clap::Arg::new("gen-config")
                .short('c')
                .long("gen-config")
                .about(
                    format!(
                        "Generate a default config file at {}",
                        config_file.to_string_lossy()
                    )
                    .as_str(),
                ),
        )
        .arg(
            clap::Arg::new("debug")
                .short('g')
                .long("debug")
                .multiple_occurrences(true)
                .hidden(true)
                .about("Enable debug-level output"),
        )
        .arg(
            clap::Arg::new("debug-log")
                .short('l')
                .long("debug-log")
                .hidden(true)
                .about("Whether to write a debug log file."),
        )
        .get_matches();

    let crate_log_level = match matches.occurrences_of("debug") {
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
        crate_name!(),
        crate_log_level
    );
    let mut log_builder = Logger::with_str(&spec)
        .format(detailed_format)
        .format_for_stderr(colored_default_format);

    if matches.is_present("debug-log") {
        let data_local_dir = dirs::data_local_dir()
            .ok_or(Error::AppDirNotFound)?
            .join(crate_name!());
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
        log_builder = log_builder
            .log_to_file()
            .suppress_timestamp()
            .directory(&log_dir);
        println!("Logging debug output to {}", log_dir.to_string_lossy());
    }

    log_builder
        .start()
        .with_context(|| "Failed to start FlexiLogger logging backend")?;

    debug!("{} version {}", crate_name!(), crate_version!());

    trace!("Loading user config");
    let conf = Config::get_config(config_file, matches.is_present("gen-config"))?;
    debug!("Configuration settings: {:?}", &conf);
    let conf_ref = Rc::new(RefCell::new(conf));

    trace!("Initializing terminal interface");
    let mut ui = term_ui::Terminal::new(conf_ref);
    trace!("Starting app");
    ui.run();
    // Explicitly drop the UI to force it to write changed settings out
    drop(ui);

    Ok(())
}
