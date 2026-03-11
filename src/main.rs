use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime};
use clap::{Parser, ValueEnum};
use eframe::egui;
use exif::{In, Reader, Tag};
use rfd::FileDialog;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(author, version, about = "按日期自动分类照片/视频到 YYYY-MM-DD 文件夹")]
struct Args {
    #[arg(short, long, help = "源目录（例如 SD 卡挂载目录）")]
    source: Option<PathBuf>,

    #[arg(short, long, help = "目标目录（会自动创建 YYYY-MM-DD 子目录）")]
    target: Option<PathBuf>,

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

    #[arg(long, default_value_t = false, help = "强制启动图形界面")]
    gui: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum TimeSource {
    Auto,
    Exif,
    Created,
    Modified,
}

impl TimeSource {
    fn label(self) -> &'static str {
        match self {
            TimeSource::Auto => "auto (元数据优先，回退文件时间)",
            TimeSource::Exif => "exif (仅元数据)",
            TimeSource::Created => "created (仅创建时间)",
            TimeSource::Modified => "modified (仅修改时间)",
        }
    }

    fn all() -> [TimeSource; 4] {
        [
            TimeSource::Auto,
            TimeSource::Exif,
            TimeSource::Created,
            TimeSource::Modified,
        ]
    }
}

#[derive(Clone, Debug)]
struct RunOptions {
    source: PathBuf,
    target: PathBuf,
    mv: bool,
    dry_run: bool,
    time_source: TimeSource,
    template: String,
}

#[derive(Debug)]
struct RunSummary {
    scanned: usize,
    handled: usize,
    skipped: usize,
    failed: usize,
}

enum WorkerMessage {
    Log(String),
    Done(Result<RunSummary, String>),
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.gui || args.source.is_none() || args.target.is_none() {
        return launch_gui();
    }

    let opts = RunOptions {
        source: args.source.expect("source checked"),
        target: args.target.expect("target checked"),
        mv: args.mv,
        dry_run: args.dry_run,
        time_source: args.time_source,
        template: args.template,
    };

    let summary = process_files(&opts, |line| println!("{line}"))?;
    println!(
        "\n完成: 扫描 {} 个文件，处理 {} 个媒体文件，跳过 {} 个非媒体文件，失败 {} 个文件。",
        summary.scanned, summary.handled, summary.skipped, summary.failed
    );
    Ok(())
}

fn launch_gui() -> Result<()> {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "FileCopyer GUI",
        native_options,
        Box::new(|cc| {
            setup_cjk_fonts(&cc.egui_ctx);
            Ok(Box::new(FileCopyerApp::default()))
        }),
    )
    .map_err(|err| anyhow::anyhow!("GUI 启动失败: {err}"))
}

fn setup_cjk_fonts(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
    ];

    let Some(font_bytes) = candidates.iter().find_map(|path| fs::read(path).ok()) else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "cjk".to_string(),
        egui::FontData::from_owned(font_bytes).into(),
    );

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "cjk".to_string());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "cjk".to_string());

    ctx.set_fonts(fonts);
}

struct FileCopyerApp {
    source: String,
    target: String,
    template: String,
    time_source: TimeSource,
    mv: bool,
    dry_run: bool,
    running: bool,
    logs: Vec<String>,
    summary: Option<String>,
    rx: Option<Receiver<WorkerMessage>>,
}

impl Default for FileCopyerApp {
    fn default() -> Self {
        Self {
            source: String::new(),
            target: String::new(),
            template: "{YYYY}-{MM}-{DD}".to_string(),
            time_source: TimeSource::Auto,
            mv: false,
            dry_run: false,
            running: false,
            logs: Vec::new(),
            summary: None,
            rx: None,
        }
    }
}

impl eframe::App for FileCopyerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("FileCopyer - 照片/视频自动归档");
            ui.label("选择源目录和目标目录，按日期自动分类。");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("源目录:");
                ui.text_edit_singleline(&mut self.source);
                if ui.button("选择...").clicked()
                    && !self.running
                    && let Some(path) = FileDialog::new().pick_folder()
                {
                    self.source = path.display().to_string();
                }
            });

            ui.horizontal(|ui| {
                ui.label("目标目录:");
                ui.text_edit_singleline(&mut self.target);
                if ui.button("选择...").clicked()
                    && !self.running
                    && let Some(path) = FileDialog::new().pick_folder()
                {
                    self.target = path.display().to_string();
                }
            });

            ui.horizontal(|ui| {
                ui.label("目录模板:");
                ui.text_edit_singleline(&mut self.template);
            });
            ui.small("可用变量: {YYYY} {MM} {DD}，例如 {YYYY}/{MM}/{DD}");

            ui.horizontal(|ui| {
                ui.label("时间来源:");
                egui::ComboBox::from_id_salt("time_source")
                    .selected_text(self.time_source.label())
                    .show_ui(ui, |ui| {
                        for source in TimeSource::all() {
                            ui.selectable_value(&mut self.time_source, source, source.label());
                        }
                    });
            });

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.mv, "移动文件（默认复制）");
                ui.checkbox(&mut self.dry_run, "仅预演（dry-run）");
            });

            ui.separator();
            ui.horizontal(|ui| {
                let start_enabled = !self.running;
                if ui
                    .add_enabled(start_enabled, egui::Button::new("开始整理"))
                    .clicked()
                {
                    self.start_task();
                }

                if self.running {
                    ui.label("处理中...");
                }
            });

            if let Some(summary) = &self.summary {
                ui.separator();
                ui.label(summary);
            }

            ui.separator();
            ui.label("执行日志:");
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.logs {
                        ui.label(line);
                    }
                });
        });
    }
}

impl FileCopyerApp {
    fn start_task(&mut self) {
        self.summary = None;

        let source = self.source.trim();
        let target = self.target.trim();
        if source.is_empty() || target.is_empty() {
            self.logs
                .push("[error] 源目录和目标目录都不能为空".to_string());
            return;
        }

        let options = RunOptions {
            source: PathBuf::from(source),
            target: PathBuf::from(target),
            mv: self.mv,
            dry_run: self.dry_run,
            time_source: self.time_source,
            template: self.template.clone(),
        };

        let (tx, rx) = mpsc::channel::<WorkerMessage>();
        self.rx = Some(rx);
        self.running = true;
        self.logs.clear();

        thread::spawn(move || {
            let result = process_files(&options, |line| {
                let _ = tx.send(WorkerMessage::Log(line));
            })
            .map_err(|err| err.to_string());

            let _ = tx.send(WorkerMessage::Done(result));
        });
    }

    fn poll_worker(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };

        while let Ok(msg) = rx.try_recv() {
            match msg {
                WorkerMessage::Log(line) => self.logs.push(line),
                WorkerMessage::Done(result) => {
                    self.running = false;
                    match result {
                        Ok(summary) => {
                            self.summary = Some(format!(
                                "完成: 扫描 {}，处理 {}，跳过 {}，失败 {}。",
                                summary.scanned, summary.handled, summary.skipped, summary.failed
                            ));
                        }
                        Err(err) => {
                            self.summary = Some(format!("执行失败: {err}"));
                        }
                    }
                }
            }
        }
    }
}

fn process_files<F>(opts: &RunOptions, mut log: F) -> Result<RunSummary>
where
    F: FnMut(String),
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
            log(format!("[skip] 无法确定日期: {}", src.display()));
            continue;
        };

        let day_dir = match render_date_template(&opts.target, &opts.template, date) {
            Ok(v) => v,
            Err(err) => {
                failed += 1;
                log(format!(
                    "[skip] 目录模板生成失败: {} ({})",
                    src.display(),
                    err
                ));
                continue;
            }
        };

        let dst = match build_unique_destination(&day_dir, src) {
            Ok(v) => v,
            Err(err) => {
                failed += 1;
                log(format!(
                    "[skip] 目标路径构建失败: {} ({})",
                    src.display(),
                    err
                ));
                continue;
            }
        };

        if opts.dry_run {
            log(format!("[dry-run] {} -> {}", src.display(), dst.display()));
            handled += 1;
            continue;
        }

        if let Err(err) = fs::create_dir_all(&day_dir)
            .with_context(|| format!("无法创建日期目录: {}", day_dir.display()))
        {
            failed += 1;
            log(format!("[skip] {}", err));
            continue;
        }

        if opts.mv {
            let move_result = fs::rename(src, &dst).or_else(|_| {
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
            });

            match move_result {
                Ok(_) => {
                    handled += 1;
                    log(format!("[move] {} -> {}", src.display(), dst.display()));
                }
                Err(err) => {
                    failed += 1;
                    log(format!("[skip] {}", err));
                }
            }
        } else {
            match fs::copy(src, &dst)
                .with_context(|| format!("复制失败: {} -> {}", src.display(), dst.display()))
            {
                Ok(_) => {
                    handled += 1;
                    log(format!("[copy] {} -> {}", src.display(), dst.display()));
                }
                Err(err) => {
                    failed += 1;
                    log(format!("[skip] {}", err));
                }
            }
        }
    }

    Ok(RunSummary {
        scanned,
        handled,
        skipped,
        failed,
    })
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
