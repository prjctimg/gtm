// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
//
// This is free software released under the GPL-3.0 license.

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::read_from_path;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, Tag};

#[cfg(test)]
use std::process::{Command, Stdio};

#[cfg(test)]
use lofty::file::FileType;

#[derive(Debug, Clone, Default)]
pub struct MetadataToWrite {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
}

pub fn write_tags(
    path: &str,
    meta: &MetadataToWrite,
    cover: Option<(Vec<u8>, String)>,
) -> Result<(), String> {
    let mut tagged_file = read_from_path(path).map_err(|e| format!("read tags: {e}"))?;

    if tagged_file.primary_tag_mut().is_none() {
        let tag_type = tagged_file.primary_tag_type();
        tagged_file.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged_file
        .primary_tag_mut()
        .ok_or_else(|| "no writable tag".to_string())?;

    if !meta.title.is_empty() {
        tag.set_title(meta.title.clone());
    }
    if !meta.artist.is_empty() {
        tag.set_artist(meta.artist.clone());
    }
    if !meta.album.is_empty() {
        tag.set_album(meta.album.clone());
    }
    if let Some(ref genre) = meta.genre {
        if !genre.is_empty() {
            tag.set_genre(genre.clone());
        }
    }
    if let Some(year) = meta.year {
        if (1000..=9999).contains(&year) {
            tag.set_date(Timestamp {
                year: year as u16,
                month: None,
                day: None,
                hour: None,
                minute: None,
                second: None,
            });
        }
    }
    if let Some(track_number) = meta.track_number {
        if track_number > 0 {
            tag.set_track(track_number as u32);
        }
    }

    if let Some((bytes, mime)) = cover {
        tag.remove_picture_type(PictureType::CoverFront);
        if !bytes.is_empty() {
            let mime_type = match mime.as_str() {
                "image/png" => MimeType::Png,
                "image/gif" => MimeType::Gif,
                _ => MimeType::Jpeg,
            };
            let picture = Picture::unchecked(bytes)
                .pic_type(PictureType::CoverFront)
                .mime_type(mime_type)
                .build();
            tag.push_picture(picture);
        }
    }

    tagged_file
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| format!("save tags: {e}"))
}

#[cfg(test)]
fn is_writable_ext(ext: &str) -> bool {
    matches!(
        FileType::from_ext(ext),
        Some(
            FileType::Mpeg
                | FileType::Flac
                | FileType::Vorbis
                | FileType::Mp4
                | FileType::Wav
                | FileType::Aiff
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_to_write_default() {
        let meta = MetadataToWrite::default();
        assert!(meta.title.is_empty());
        assert!(meta.year.is_none());
    }

    #[test]
    fn test_writable_formats_supported() {
        for ext in ["mp3", "flac", "ogg", "m4a", "mp4", "wav", "aiff"] {
            assert!(is_writable_ext(ext), "{ext} should support tag writes");
        }
        assert!(!is_writable_ext("txt"));
    }

    #[test]
    fn test_roundtrip_write_and_read() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("gtm_tags_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.wav");

        let out = Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                &path.to_string_lossy(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .unwrap();
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let meta = MetadataToWrite {
            title: "Beautiful".to_string(),
            artist: "Bazzi".to_string(),
            album: "Cosmic Latte".to_string(),
            genre: Some("Pop".to_string()),
            year: Some(2018),
            track_number: Some(4),
        };
        let write_result = write_tags(&path.to_string_lossy(), &meta, None);
        assert!(write_result.is_ok(), "write failed: {write_result:?}");

        let tagged = read_from_path(&path).unwrap();
        let tag = tagged.primary_tag().unwrap();
        assert_eq!(tag.title().as_deref(), Some("Beautiful"));
        assert_eq!(tag.artist().as_deref(), Some("Bazzi"));
        assert_eq!(tag.album().as_deref(), Some("Cosmic Latte"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
