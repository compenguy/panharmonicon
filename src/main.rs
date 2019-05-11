#[macro_use]
extern crate log;

#[macro_use]
extern crate clap;

mod errors;
use errors::{Result, Error};

use std::boxed::Box;

fn main() -> Result<()> {
    let matches = app_from_crate!("")
        .setting(clap::AppSettings::ColorAuto)
        .setting(clap::AppSettings::ColoredHelp)
        .arg(
            clap::Arg::with_name("gen-config")
                .short("c")
                .long("gen-config")
        ).arg(
            clap::Arg::with_name("debug")
                .short("g")
                .long("debug")
                .multiple(true)
                .hidden(true)
                .help("Enable debug-level output"),
        ).get_matches();

    loggerv::init_with_verbosity(matches.occurrences_of("debug")).map_err(|e| Error::LoggerError(Box::new(e)))?;

    debug!("{} version {}", crate_name!(), crate_version!());
    debug!("{:<10} {}", "OS:", sys_info::os_type().unwrap_or_else(|_| String::from("Unknown")));
    debug!("{:<10} {}", "Release:", sys_info::os_release().unwrap_or_else(|_| String::from("Unknown")));
    debug!("{:<10} {}", "Host:", sys_info::hostname().unwrap_or_else(|_| String::from("Unknown")));

    // TODO: process gen-config option, then load config
    let _config_file = dirs::config_dir().ok_or_else(|| Error::ConfigDirNotFound)?.join(crate_name!()).join("config.json");

    Ok(())
}
