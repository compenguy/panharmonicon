use clap::{app_from_crate, crate_authors, crate_description, crate_name, crate_version};
use log::{debug, trace};
use termion::input::TermRead;

mod errors;
use errors::{Error, Result};

mod config;
use config::Config;

mod term;
use term::TerminalWin;

mod logger;
use logger::LogPane;

mod pandora;
use pandora::PandoraPane;

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
    tui_logger::init_logger(max_log_level).map_err(|e| Error::LoggerFailure(Box::new(e)))?;
    tui_logger::set_default_level(log_level);

    if let Some(log_file) = matches.value_of("debug-log") {
        tui_logger::set_log_file(log_file).map_err(|e| Error::LoggerFileFailure(Box::new(e)))?;
    }

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

    let mut term = TerminalWin::new(conf_ref.clone())?;
    let log = LogPane::new(conf_ref.clone())?;
    term.add_pane(log)?;
    let pandora = PandoraPane::new(conf_ref.clone())?;
    term.add_pane(pandora)?;

    let stdin = std::io::stdin();
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        for c in stdin.events() {
            trace!(target:"INPUT", "Stdin event received {:?}", c);
            // TODO: Error handling
            tx.send(c.unwrap()).unwrap();
        }
    });
    // Main event loop
    'main: loop {
        // Process all pending input events
        for evt in rx.try_iter() {
            trace!(target: "Event rx", "{:?}", evt);
            if let termion::event::Event::Key(key) = evt {
                match key {
                    termion::event::Key::Char('q') => break 'main,
                    _ => trace!(target: "Key rx", "Unhandled key event {:?}", key),
                }
            }
        }

        term.render()?;
        thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(())
}
