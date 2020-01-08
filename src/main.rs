use std::boxed::Box;
use std::cell::RefCell;
use std::rc::Rc;

use clap::{app_from_crate, crate_authors, crate_description, crate_name, crate_version};
use flexi_logger::Logger;
use log::{debug, error, trace};

mod errors;
use crate::errors::{Error, Result};

mod config;
use crate::config::Config;

mod crossterm;
use crate::crossterm as term;

mod app;

fn main() -> Result<()> {
    let config_file = dirs::config_dir()
        .ok_or_else(|| Error::AppDirNotFound)?
        .join(crate_name!())
        .join("config.json");
    let matches = app_from_crate!("")
        .setting(clap::AppSettings::ColorAuto)
        .setting(clap::AppSettings::ColoredHelp)
        .arg(
            clap::Arg::with_name("gen-config")
                .short("c")
                .long("gen-config")
                .help(
                    format!(
                        "Generate a default config file at {}",
                        config_file.to_string_lossy()
                    )
                    .as_str(),
                ),
        )
        .arg(
            clap::Arg::with_name("debug")
                .short("g")
                .long("debug")
                .multiple(true)
                .hidden(true)
                .help("Enable debug-level output"),
        )
        .arg(
            clap::Arg::with_name("debug-log")
                .short("l")
                .long("debug-log")
                .hidden(true)
                .help("Whether to write a debug log file."),
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
    let mut log_builder = Logger::with_str(&spec);
    //let mut log_builder = Logger::with(flexi_logger::LogSpecification::default(log_level).build());

    if matches.is_present("debug-log") {
        let data_local_dir = dirs::data_local_dir()
            .ok_or_else(|| Error::AppDirNotFound)?
            .join(crate_name!());
        let log_dir = data_local_dir
            .join("logs")
            .join(format!("{}", chrono::offset::Utc::now()));
        if !log_dir.is_dir() {
            std::fs::create_dir_all(&log_dir)
                .map_err(|e| Error::AppDirCreateFailure(Box::new(e)))?;
        }
        log_builder = log_builder
            .log_to_file()
            .suppress_timestamp()
            .directory(&log_dir);
    }

    log_builder
        .start()
        .map_err(|e| Error::FlexiLoggerFailure(Box::new(e)))?;

    debug!("{} version {}", crate_name!(), crate_version!());
    debug!(
        "{:<10} {}",
        "OS:",
        sys_info::os_type().unwrap_or_else(|_| String::from("Unknown"))
    );
    debug!(
        "{:<10} {}",
        "Release:",
        sys_info::os_release().unwrap_or_else(|_| String::from("Unknown"))
    );
    debug!(
        "{:<10} {}",
        "Host:",
        sys_info::hostname().unwrap_or_else(|_| String::from("Unknown"))
    );

    trace!("Loading user config");
    let conf = Config::get_config(config_file, matches.is_present("gen-config"))?;
    let conf_ref = Rc::new(RefCell::new(conf));

    let session = term::Terminal::new(conf_ref.clone());
    trace!("Initializing app interface");
    let mut app = app::Panharmonicon::new(conf_ref, session);
    trace!("Starting app");
    while let Err(e) = app.run() {
        error!("{:?}", e);
    }

    Ok(())
}
