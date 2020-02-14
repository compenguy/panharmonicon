use std::path::{Path, PathBuf};

use clap::crate_name;
use log::trace;
use reqwest;

pub use pandora_api::json::user::Station;

use crate::app::{Audio, Playing, SongInfo};
use crate::errors::{Error, Result};

pub(crate) fn get_cached_media(playing: &Playing, audio: Audio) -> Result<PathBuf> {
    trace!("Caching active track {}", playing.track_token);
    // Adjust track metadata so that it's path/filename-safe
    let artist_filename = filename_formatter(&playing.info.artist);
    let album_filename = filename_formatter(&playing.info.album);
    let song_filename = filename_formatter(&playing.info.name);
    let filename = format!(
        "{} - {}.{}",
        artist_filename,
        song_filename,
        audio.get_extension()
    );

    // Construct full path to the cached file
    let cache_file = dirs::cache_dir()
        .ok_or_else(|| Error::AppDirNotFound)?
        .join(crate_name!())
        .join(artist_filename)
        .join(album_filename)
        .join(filename);

    // Check cache, and if track isn't in the cache, add it
    if cache_file.exists() {
        trace!("Song already in cache.");
    } else {
        trace!("Caching song.");
        let tempdir = dirs::cache_dir()
            .ok_or_else(|| Error::AppDirNotFound)?
            .join(crate_name!())
            .join("tmp");
        // Ensure that target directory exists
        if !tempdir.exists() {
            std::fs::create_dir_all(&tempdir).map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
        }
        let tempdest = mktemp::Temp::new_file_in(tempdir)
            .map_err(|e| Error::FileWriteFailure(Box::new(e)))?
            .release();
        save_url_to_file(&audio.url, &tempdest)?;
        tag_mp3(&playing.info, &tempdest)?;
        if let Some(cache_parent_dir) = cache_file.parent() {
            if !cache_parent_dir.exists() {
                std::fs::create_dir_all(&cache_parent_dir)
                    .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
            }
        }
        std::fs::rename(&tempdest, &cache_file)
            .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
        trace!("Song added to cache.");
    }
    Ok(cache_file)
}

fn tag_mp3<P: AsRef<Path>>(metadata: &SongInfo, path: P) -> Result<()> {
    trace!("Reading tags from mp3");
    let mut tag = match id3::Tag::read_from_path(&path) {
        Ok(tag) => tag,
        Err(id3::Error {
            kind: id3::ErrorKind::NoTag,
            ..
        }) => id3::Tag::new(),
        err => err?,
    };

    // TODO: pipe in replay gain value, and create frame for the:
    //   * RVA2 tag (if using v2.4)
    //   * XRVA tag (http://id3.org/Experimental%20RVA2)
    //   * http://id3.org/id3v2.4.0-frames section 4.11

    trace!("Updating tags with filesystem metadata");
    if tag.artist().is_none() {
        tag.set_artist(&metadata.artist);
    }
    if tag.album().is_none() {
        tag.set_album(&metadata.album);
    }
    if tag.title().is_none() {
        tag.set_title(&metadata.name);
    }

    trace!("Writing tags back to file");
    tag.write_to_path(&path, id3::Version::Id3v24)
        .map_err(Error::from)
}

fn save_url_to_file<P: AsRef<Path>>(url: &str, path: P) -> Result<()> {
    let mut resp = reqwest::blocking::get(url)?;

    let file = std::fs::File::create(path).map_err(|e| Error::FileWriteFailure(Box::new(e)))?;

    resp.copy_to(&mut std::io::BufWriter::new(file))
        .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
    Ok(())
}

fn filename_formatter(text: &str) -> String {
    text.replace("/", "_")
        .replace("\\", "_")
        .replace(":", "_")
        .replace("-", "_")
}
