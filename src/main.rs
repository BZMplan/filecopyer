use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Local, NaiveDateTime};
use clap::Parser;
use exif::{In, Reader, Tag};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "按日期自动分类照片/视频到 YYYY-MM-DD 文件夹"
)]
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

        let date = detect_date(src)
            .or_else(|| detect_fs_date(src))
            .with_context(|| format!("无法读取文件日期: {}", src.display()))?;

        let folder_name = format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day());
        let day_dir = args.target.join(folder_name);
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
                        format!("跨设备移动失败，复制也失败: {} -> {}", src.display(), dst.display())
                    })
                    .and_then(|_| {
                        fs::remove_file(src).with_context(|| format!("删除源文件失败: {}", src.display()))
                    })
            })?;
            println!("[move] {} -> {}", src.display(), dst.display());
        } else {
            fs::copy(src, &dst).with_context(|| {
                format!("复制失败: {} -> {}", src.display(), dst.display())
            })?;
            println!("[copy] {} -> {}", src.display(), dst.display());
        }
        handled += 1;
    }

    println!(
        "\n完成: 扫描 {} 个文件，处理 {} 个媒体文件，跳过 {} 个非媒体文件。",
        scanned, handled, skipped
    );
    Ok(())
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

fn detect_date(path: &Path) -> Option<NaiveDateTime> {
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

fn detect_fs_date(path: &Path) -> Option<NaiveDateTime> {
    let meta = fs::metadata(path).ok()?;

    if let Ok(created) = meta.created() {
        let dt: DateTime<Local> = created.into();
        return Some(dt.naive_local());
    }
    if let Ok(modified) = meta.modified() {
        let dt: DateTime<Local> = modified.into();
        return Some(dt.naive_local());
    }
    None
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
