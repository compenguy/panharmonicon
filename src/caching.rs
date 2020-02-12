use std::io::{Read, Seek, Write};
use std::path::PathBuf;

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
        // Control the scope of temp_rw, so that we control when it closes
        {
            let mut temp_rw = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&tempdest)
                .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
            save_url_to_writer(&audio.url, &temp_rw)?;
            tag_mp3(&mut temp_rw, &playing.info)?;
        }
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

fn tag_mp3<F: Read + Write + Seek>(mut mp3_rw: &mut F, metadata: &SongInfo) -> Result<()> {
    mp3_rw
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| Error::FileReadFailure(Box::new(e)))?;
    trace!("Reading tags from mp3");
    let mut tag = match id3::Tag::read_from(&mut mp3_rw) {
        Err(id3::Error {
            kind: id3::ErrorKind::NoTag,
            ..
        }) => id3::Tag::new(),
        Ok(tag) => tag,
        err => err?,
    };

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
    mp3_rw
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
    tag.write_to(&mut mp3_rw, id3::Version::Id3v23)
        .map_err(Error::from)
}

fn save_url_to_writer<W: Write>(url: &str, writer: W) -> Result<()> {
    let mut buf_writer = std::io::BufWriter::new(writer);
    let mut resp = reqwest::blocking::get(url).map_err(Error::from)?;
    resp.copy_to(&mut buf_writer)
        .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
    Ok(())
}

fn filename_formatter(text: &str) -> String {
    text.replace("/", "_")
        .replace("\\", "_")
        .replace(":", "_")
        .replace("-", "_")
}
