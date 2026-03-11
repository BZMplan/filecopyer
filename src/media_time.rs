use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime};
use exif::{In, Reader, Tag};

use crate::types::TimeSource;

pub fn detect_date_by_source(path: &Path, source: TimeSource) -> Option<NaiveDateTime> {
    match source {
        TimeSource::Auto => detect_media_metadata_date(path)
            .or_else(|| detect_fs_created_date(path))
            .or_else(|| detect_fs_modified_date(path)),
        TimeSource::Exif => detect_media_metadata_date(path),
        TimeSource::Created => detect_fs_created_date(path),
        TimeSource::Modified => detect_fs_modified_date(path),
    }
}

pub fn is_media_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .map(|s| s.to_ascii_lowercase());

    matches!(
        ext.as_deref(),
        Some("jpg")
            | Some("jpeg")
            | Some("png")
            | Some("heic")
            | Some("heif")
            | Some("gif")
            | Some("tif")
            | Some("tiff")
            | Some("bmp")
            | Some("webp")
            | Some("raw")
            | Some("cr2")
            | Some("cr3")
            | Some("nef")
            | Some("nrw")
            | Some("arw")
            | Some("dng")
            | Some("mp4")
            | Some("mov")
            | Some("m4v")
            | Some("avi")
            | Some("mkv")
            | Some("mts")
            | Some("3gp")
    )
}

fn detect_media_metadata_date(path: &Path) -> Option<NaiveDateTime> {
    detect_exif_date(path).or_else(|| detect_video_metadata_date(path))
}

fn detect_exif_date(path: &Path) -> Option<NaiveDateTime> {
    let file = File::open(path).ok()?;
    let mut buf = BufReader::new(file);
    let exif = Reader::new().read_from_container(&mut buf).ok()?;

    let tags = [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime];
    for tag in tags {
        if let Some(field) = exif.get_field(tag, In::PRIMARY) {
            let raw = field.display_value().with_unit(&exif).to_string();
            if let Some(dt) = parse_exif_datetime(&raw) {
                return Some(dt);
            }
        }
    }
    None
}

fn parse_exif_datetime(s: &str) -> Option<NaiveDateTime> {
    let value = s.trim().split('\0').next()?.trim();
    NaiveDateTime::parse_from_str(value, "%Y:%m:%d %H:%M:%S").ok()
}

fn detect_fs_created_date(path: &Path) -> Option<NaiveDateTime> {
    let meta = fs::metadata(path).ok()?;
    meta.created().ok().map(|created| {
        let dt: DateTime<Local> = created.into();
        dt.naive_local()
    })
}

fn detect_fs_modified_date(path: &Path) -> Option<NaiveDateTime> {
    let meta = fs::metadata(path).ok()?;
    meta.modified().ok().map(|modified| {
        let dt: DateTime<Local> = modified.into();
        dt.naive_local()
    })
}

fn detect_video_metadata_date(path: &Path) -> Option<NaiveDateTime> {
    if !is_quicktime_compatible_video(path) {
        return None;
    }
    parse_quicktime_creation_time(path)
}

fn is_quicktime_compatible_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(OsStr::to_str)
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("mp4") | Some("mov") | Some("m4v") | Some("3gp")
    )
}

fn parse_quicktime_creation_time(path: &Path) -> Option<NaiveDateTime> {
    let mut file = File::open(path).ok()?;
    let file_size = file.metadata().ok()?.len();
    find_qt_creation_time(&mut file, 0, file_size, 0)
}

fn find_qt_creation_time(
    file: &mut File,
    start: u64,
    end: u64,
    depth: usize,
) -> Option<NaiveDateTime> {
    if depth > 8 || start >= end {
        return None;
    }

    let mut cursor = start;
    while cursor + 8 <= end {
        file.seek(SeekFrom::Start(cursor)).ok()?;

        let mut size_buf = [0u8; 4];
        let mut kind = [0u8; 4];
        file.read_exact(&mut size_buf).ok()?;
        file.read_exact(&mut kind).ok()?;

        let mut atom_size = u32::from_be_bytes(size_buf) as u64;
        let mut header_size = 8u64;

        if atom_size == 1 {
            let mut ext_buf = [0u8; 8];
            file.read_exact(&mut ext_buf).ok()?;
            atom_size = u64::from_be_bytes(ext_buf);
            header_size = 16;
        } else if atom_size == 0 {
            atom_size = end.saturating_sub(cursor);
        }

        if atom_size < header_size {
            return None;
        }

        let payload_start = cursor + header_size;
        let payload_end = cursor + atom_size;
        if payload_end > end {
            return None;
        }

        if kind == *b"mvhd" || kind == *b"mdhd" {
            if let Some(dt) = parse_qt_fullbox_creation_time(file, payload_start, payload_end) {
                return Some(dt);
            }
        } else if is_qt_container(kind) {
            let nested = if kind == *b"meta" && payload_end.saturating_sub(payload_start) >= 4 {
                find_qt_creation_time(file, payload_start + 4, payload_end, depth + 1)
            } else {
                find_qt_creation_time(file, payload_start, payload_end, depth + 1)
            };
            if let Some(dt) = nested {
                return Some(dt);
            }
        }

        cursor = payload_end;
    }

    None
}

fn is_qt_container(kind: [u8; 4]) -> bool {
    matches!(
        &kind,
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"udta" | b"meta"
    )
}

fn parse_qt_fullbox_creation_time(
    file: &mut File,
    payload_start: u64,
    payload_end: u64,
) -> Option<NaiveDateTime> {
    if payload_end.saturating_sub(payload_start) < 8 {
        return None;
    }

    file.seek(SeekFrom::Start(payload_start)).ok()?;
    let mut fullbox = [0u8; 4];
    file.read_exact(&mut fullbox).ok()?;
    let version = fullbox[0];

    let secs = if version == 1 {
        if payload_end.saturating_sub(payload_start) < 20 {
            return None;
        }
        let mut buf = [0u8; 8];
        file.read_exact(&mut buf).ok()?;
        u64::from_be_bytes(buf)
    } else {
        let mut buf = [0u8; 4];
        file.read_exact(&mut buf).ok()?;
        u32::from_be_bytes(buf) as u64
    };

    qt_seconds_to_naive_datetime(secs)
}

fn qt_seconds_to_naive_datetime(seconds_since_1904: u64) -> Option<NaiveDateTime> {
    if seconds_since_1904 > i64::MAX as u64 {
        return None;
    }

    let epoch = NaiveDate::from_ymd_opt(1904, 1, 1)?.and_hms_opt(0, 0, 0)?;
    epoch.checked_add_signed(Duration::seconds(seconds_since_1904 as i64))
}
