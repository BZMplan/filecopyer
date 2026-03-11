use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime};
use clap::{Parser, ValueEnum};
use exif::{In, Reader, Tag};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(author, version, about = "按日期自动分类照片/视频到 YYYY-MM-DD 文件夹")]
struct Args {
    #[arg(short, long, help = "源目录（例如 SD 卡挂载目录）")]
    source: PathBuf,

    #[arg(short, long, help = "目标目录（会自动创建 YYYY-MM-DD 子目录）")]
    target: PathBuf,

    #[arg(
        short,
        long,
        default_value_t = false,
        help = "实际移动文件（默认是复制）"
    )]
    mv: bool,

    #[arg(short, long, default_value_t = false, help = "仅打印计划，不执行写入")]
    dry_run: bool,

    #[arg(
        long,
        value_enum,
        default_value_t = TimeSource::Auto,
        help = "日期来源策略：auto/exif/created/modified"
    )]
    time_source: TimeSource,

    #[arg(
        long,
        default_value = "{YYYY}-{MM}-{DD}",
        help = "目标目录模板，支持 {YYYY} {MM} {DD}（可包含子目录）"
    )]
    template: String,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TimeSource {
    Auto,
    Exif,
    Created,
    Modified,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if !args.source.exists() {
        anyhow::bail!("源目录不存在: {}", args.source.display());
    }
    if !args.source.is_dir() {
        anyhow::bail!("源路径不是目录: {}", args.source.display());
    }
    if !args.target.exists() && !args.dry_run {
        fs::create_dir_all(&args.target)
            .with_context(|| format!("无法创建目标目录: {}", args.target.display()))?;
    }

    let mut scanned = 0usize;
    let mut handled = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for entry in WalkDir::new(&args.source)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let src = entry.path();
        scanned += 1;

        if !is_media_file(src) {
            skipped += 1;
            continue;
        }

        let Some(date) = detect_date_by_source(src, args.time_source) else {
            eprintln!("[skip] 无法确定日期: {}", src.display());
            failed += 1;
            continue;
        };

        let day_dir = match render_date_template(&args.target, &args.template, date) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("[skip] 目录模板生成失败: {} ({})", src.display(), err);
                failed += 1;
                continue;
            }
        };
        let dst = build_unique_destination(&day_dir, src)?;

        if args.dry_run {
            println!("[dry-run] {} -> {}", src.display(), dst.display());
            handled += 1;
            continue;
        }

        fs::create_dir_all(&day_dir)
            .with_context(|| format!("无法创建日期目录: {}", day_dir.display()))?;

        if args.mv {
            fs::rename(src, &dst).or_else(|_| {
                fs::copy(src, &dst)
                    .with_context(|| {
                        format!(
                            "跨设备移动失败，复制也失败: {} -> {}",
                            src.display(),
                            dst.display()
                        )
                    })
                    .and_then(|_| {
                        fs::remove_file(src)
                            .with_context(|| format!("删除源文件失败: {}", src.display()))
                    })
            })?;
            println!("[move] {} -> {}", src.display(), dst.display());
        } else {
            fs::copy(src, &dst)
                .with_context(|| format!("复制失败: {} -> {}", src.display(), dst.display()))?;
            println!("[copy] {} -> {}", src.display(), dst.display());
        }
        handled += 1;
    }

    println!(
        "\n完成: 扫描 {} 个文件，处理 {} 个媒体文件，跳过 {} 个非媒体文件，失败 {} 个文件。",
        scanned, handled, skipped, failed
    );
    Ok(())
}

fn detect_date_by_source(path: &Path, source: TimeSource) -> Option<NaiveDateTime> {
    match source {
        TimeSource::Auto => detect_media_metadata_date(path)
            .or_else(|| detect_fs_created_date(path))
            .or_else(|| detect_fs_modified_date(path)),
        TimeSource::Exif => detect_media_metadata_date(path),
        TimeSource::Created => detect_fs_created_date(path),
        TimeSource::Modified => detect_fs_modified_date(path),
    }
}

fn is_media_file(path: &Path) -> bool {
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
            if let Some(dt) = if kind == *b"meta" && payload_end.saturating_sub(payload_start) >= 4
            {
                find_qt_creation_time(file, payload_start + 4, payload_end, depth + 1)
            } else {
                find_qt_creation_time(file, payload_start, payload_end, depth + 1)
            } {
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

fn render_date_template(
    target_root: &Path,
    template: &str,
    date: NaiveDateTime,
) -> Result<PathBuf> {
    let rendered = template
        .replace("{YYYY}", &format!("{:04}", date.year()))
        .replace("{MM}", &format!("{:02}", date.month()))
        .replace("{DD}", &format!("{:02}", date.day()));

    if rendered.trim().is_empty() {
        anyhow::bail!("模板渲染后为空");
    }

    let rel = Path::new(&rendered);
    if rel.is_absolute() {
        anyhow::bail!("模板不能是绝对路径: {}", rendered);
    }

    for c in rel.components() {
        match c {
            Component::Normal(_) => {}
            _ => anyhow::bail!("模板包含非法路径片段: {}", rendered),
        }
    }

    Ok(target_root.join(rel))
}

fn build_unique_destination(day_dir: &Path, src: &Path) -> Result<PathBuf> {
    let file_name = src
        .file_name()
        .and_then(OsStr::to_str)
        .with_context(|| format!("无法解析文件名: {}", src.display()))?;

    let candidate = day_dir.join(file_name);
    if !candidate.exists() {
        return Ok(candidate);
    }

    let stem = src
        .file_stem()
        .and_then(OsStr::to_str)
        .with_context(|| format!("无法解析文件名 stem: {}", src.display()))?;
    let ext = src.extension().and_then(OsStr::to_str).unwrap_or("");

    for i in 1..=9999 {
        let new_name = if ext.is_empty() {
            format!("{}_{}", stem, i)
        } else {
            format!("{}_{}.{}", stem, i, ext)
        };
        let path = day_dir.join(new_name);
        if !path.exists() {
            return Ok(path);
        }
    }

    anyhow::bail!("同名文件过多，无法生成唯一文件名: {}", src.display())
}
