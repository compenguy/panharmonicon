use clap::{app_from_crate, crate_authors, crate_description, crate_name, crate_version};
use flexi_logger::Logger;
use log::{debug, trace};
use mktemp::TempDir;

mod errors;
use errors::{Error, Result};

mod config;
use config::Config;

mod panharmonicon;

use std::boxed::Box;
use std::cell::RefCell;
use std::rc::Rc;
use std::thread;

fn main() -> Result<()> {
    let config_file = dirs::config_dir()
        .ok_or_else(|| Error::ConfigDirNotFound)?
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
                .help("File to write log output to."),
        )
        .get_matches();

    let log_level = match matches.occurrences_of("debug") {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };
    // Unless the user specifically requested debug or trace level logging, we cap
    // logging level at Info
    let max_log_level = std::cmp::max(log::LevelFilter::Info, log_level);
    let mut log_builder = Logger::with_env();

    if let Some(log_file) = matches.value_of("debug-log") {
        let td = TempDir::new(crate_name!()).map_err(|e| Error::LoggerFileFailure(Box::new(e)))?;
        log_builder = log_builder
            .log_to_file()
            .suppress_timestamp()
            .directory(td.path());
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

    let conf = Config::get_config(config_file, matches.is_present("gen-config"))?;
    let conf_ref = Rc::new(RefCell::new(conf));

    let mut panharmonicon = panharmonicon::Panharmonicon::new(conf_ref.clone())?;

    panharmonicon.run();

    Ok(())
}
