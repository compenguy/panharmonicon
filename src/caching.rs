use std::path::{Path, PathBuf};

use clap::crate_name;
use log::{error, trace};
use pandora_api::json::station::PlaylistTrack;

use crate::errors::{Error, Result};

// https://en.wikipedia.org/wiki/Filename#Reserved_characters_and_words
fn sanitize_filename(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '/' => '_',
            '\\' => '_',
            '?' => '_',
            '*' => '_',
            ':' => '_',
            '|' => '_',
            '<' => '_',
            '>' => '_',
            _ => c,
        })
        .collect()
}

fn app_cache_dir() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .ok_or_else(|| Error::AppDirNotFound)?
        .join(crate_name!()))
}

fn precached_path_for_track(track: &PlaylistTrack) -> Option<PathBuf> {
    if let Some(serde_json::value::Value::String(path_str)) = track.optional.get("cached") {
        let path = PathBuf::from(path_str);
        if path.exists() {
            return Some(path);
        } else {
            trace!(
                "Marked as precached, but doesn't exist: {}",
                path.to_string_lossy()
            );
        }
    }
    None
}

fn cached_path_for_track(track: &PlaylistTrack, create_path: bool) -> Result<PathBuf> {
    if let Some(precached) = precached_path_for_track(track) {
        return Ok(precached);
    }

    let artist = sanitize_filename(&track.artist_name);
    let album = sanitize_filename(&track.album_name);
    let song = sanitize_filename(&track.song_name);

    let mut track_cache_path = app_cache_dir()?.join(&artist).join(&album);

    if create_path {
        std::fs::create_dir_all(&track_cache_path)
            .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
    }
    let filename = format!("{} - {}.{}", artist, song, "mp3");
    track_cache_path.push(filename);
    Ok(track_cache_path)
}

pub(crate) fn add_to_cache(track: &mut PlaylistTrack) -> Result<PathBuf> {
    if let Some(path) = precached_path_for_track(track) {
        return Ok(path);
    }

    let path = cached_path_for_track(track, true)?;

    if let Err(e) = save_url_to_file(&track.additional_audio_url, &path) {
        error!(
            "Error downloading track {} to {}: {:?}",
            &track.additional_audio_url,
            &path.to_string_lossy(),
            &e
        );
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }

    if let Err(e) = tag_mp3(&track, &path) {
        error!(
            "Error tagging track at {}: {:?}",
            path.to_string_lossy(),
            &e
        );
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }

    track.optional.insert(
        String::from("cached"),
        serde_json::value::Value::String(path.to_string_lossy().to_string()),
    );
    Ok(path)
}

fn tag_mp3<P: AsRef<Path>>(track: &PlaylistTrack, path: P) -> Result<()> {
    let id3_ver = id3::Version::Id3v23;
    trace!("Reading tags from mp3");
    let mut tag = match id3::Tag::read_from_path(&path) {
        Ok(tag) => tag,
        Err(id3::Error {
            kind: id3::ErrorKind::NoTag,
            ..
        }) => id3::Tag::new(),
        err => err?,
    };

    let duration: Option<u32> = track
        .optional
        .get("trackLength")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    // TODO: if track.replaygain parses correctly, create replaygain
    // frame for the:
    //   * RVA2 tag (if using v2.4)
    //   * XRVA tag (http://id3.org/Experimental%20RVA2)
    //   * http://id3.org/id3v2.4.0-frames section 4.11

    trace!("Updating tags with pandora metadata");
    let mut dirty = false;

    if tag.artist().is_none() {
        tag.set_artist(&track.artist_name);
        dirty = true;
    }
    if tag.album().is_none() {
        tag.set_album(&track.album_name);
        dirty = true;
    }
    if tag.title().is_none() {
        tag.set_title(&track.song_name);
        dirty = true;
    }
    if tag.duration().is_none() {
        if let Some(duration) = duration {
            tag.set_duration(duration);
            dirty = true;
        }
    }

    trace!("Writing tags back to file");
    if dirty {
        tag.write_to_path(&path, id3_ver)?;
    }
    Ok(())
}

fn save_url_to_file<P: AsRef<Path>>(url: &str, path: P) -> Result<()> {
    let mut resp = reqwest::blocking::get(url)?;

    let file = std::fs::File::create(path).map_err(|e| Error::FileWriteFailure(Box::new(e)))?;

    resp.copy_to(&mut std::io::BufWriter::new(file))
        .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
    Ok(())
}
