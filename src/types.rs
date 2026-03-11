use std::path::PathBuf;

use chrono::Local;
use clap::ValueEnum;

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum TimeSource {
    Auto,
    Exif,
    Created,
    Modified,
}

impl TimeSource {
    pub fn label(self) -> &'static str {
        match self {
            TimeSource::Auto => "auto (元数据优先，回退文件时间)",
            TimeSource::Exif => "exif (仅元数据)",
            TimeSource::Created => "created (仅创建时间)",
            TimeSource::Modified => "modified (仅修改时间)",
        }
    }

    pub fn all() -> [TimeSource; 4] {
        [
            TimeSource::Auto,
            TimeSource::Exif,
            TimeSource::Created,
            TimeSource::Modified,
        ]
    }
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub source: PathBuf,
    pub target: PathBuf,
    pub mv: bool,
    pub dry_run: bool,
    pub time_source: TimeSource,
    pub template: String,
}

#[derive(Debug)]
pub struct RunSummary {
    pub scanned: usize,
    pub handled: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Warn,
    Error,
    DryRun,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
}

impl LogEntry {
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            level,
            message: message.into(),
        }
    }

    pub fn tag(&self) -> &'static str {
        match self.level {
            LogLevel::Info => "INFO",
            LogLevel::Success => "OK",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERR",
            LogLevel::DryRun => "DRY",
        }
    }
}

pub enum WorkerMessage {
    Log(LogEntry),
    Done(Result<RunSummary, String>),
}
