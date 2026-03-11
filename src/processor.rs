use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDateTime};
use walkdir::WalkDir;

use crate::media_time::{detect_date_by_source, is_media_file};
use crate::types::{LogEntry, LogLevel, RunOptions, RunSummary};

pub fn process_files<F>(opts: &RunOptions, mut log: F) -> Result<RunSummary>
where
    F: FnMut(LogEntry),
{
    if !opts.source.exists() {
        anyhow::bail!("源目录不存在: {}", opts.source.display());
    }
    if !opts.source.is_dir() {
        anyhow::bail!("源路径不是目录: {}", opts.source.display());
    }
    if !opts.target.exists() && !opts.dry_run {
        fs::create_dir_all(&opts.target)
            .with_context(|| format!("无法创建目标目录: {}", opts.target.display()))?;
    }

    let mut scanned = 0usize;
    let mut handled = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for entry in WalkDir::new(&opts.source)
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

        let Some(date) = detect_date_by_source(src, opts.time_source) else {
            failed += 1;
            log(LogEntry::new(
                LogLevel::Warn,
                format!("无法确定日期: {}", src.display()),
            ));
            continue;
        };

        let day_dir = match render_date_template(&opts.target, &opts.template, date) {
            Ok(v) => v,
            Err(err) => {
                failed += 1;
                log(LogEntry::new(
                    LogLevel::Warn,
                    format!("目录模板生成失败: {} ({})", src.display(), err),
                ));
                continue;
            }
        };

        let dst = match build_unique_destination(&day_dir, src) {
            Ok(v) => v,
            Err(err) => {
                failed += 1;
                log(LogEntry::new(
                    LogLevel::Warn,
                    format!("目标路径构建失败: {} ({})", src.display(), err),
                ));
                continue;
            }
        };

        if opts.dry_run {
            log(LogEntry::new(
                LogLevel::DryRun,
                format!("预演：从 {} 到 {}", src.display(), dst.display()),
            ));
            handled += 1;
            continue;
        }

        if let Err(err) = fs::create_dir_all(&day_dir)
            .with_context(|| format!("无法创建日期目录: {}", day_dir.display()))
        {
            failed += 1;
            log(LogEntry::new(LogLevel::Error, format!("{err}")));
            continue;
        }

        if opts.mv {
            let move_result = fs::rename(src, &dst).or_else(|_| {
                fs::copy(src, &dst)
                    .with_context(|| {
                        format!(
                            "跨设备移动失败，复制也失败：从 {} 到 {}",
                            src.display(),
                            dst.display()
                        )
                    })
                    .and_then(|_| {
                        fs::remove_file(src)
                            .with_context(|| format!("删除源文件失败: {}", src.display()))
                    })
            });

            match move_result {
                Ok(_) => {
                    handled += 1;
                    log(LogEntry::new(
                        LogLevel::Success,
                        format!("移动：从 {} 到 {}", src.display(), dst.display()),
                    ));
                }
                Err(err) => {
                    failed += 1;
                    log(LogEntry::new(LogLevel::Error, format!("{err}")));
                }
            }
        } else {
            match fs::copy(src, &dst)
                .with_context(|| format!("复制失败：从 {} 到 {}", src.display(), dst.display()))
            {
                Ok(_) => {
                    handled += 1;
                    log(LogEntry::new(
                        LogLevel::Success,
                        format!("复制：从 {} 到 {}", src.display(), dst.display()),
                    ));
                }
                Err(err) => {
                    failed += 1;
                    log(LogEntry::new(LogLevel::Error, format!("{err}")));
                }
            }
        }
    }

    log(LogEntry::new(
        LogLevel::Info,
        format!(
            "完成 扫描={} 处理={} 跳过={} 失败={}",
            scanned, handled, skipped, failed
        ),
    ));

    Ok(RunSummary {
        scanned,
        handled,
        skipped,
        failed,
    })
}

pub fn format_log_line(entry: &LogEntry) -> String {
    format!("[{}] [{}] {}", entry.timestamp, entry.tag(), entry.message)
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
