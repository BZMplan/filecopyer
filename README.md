# FileCopyer

按日期自动整理照片和视频的 Rust 工具，支持命令行和图形界面（GUI）。

## 功能

- 递归扫描源目录中的照片/视频文件
- 按日期自动创建目标目录并分类（默认 `YYYY-MM-DD`）
- 支持自定义目录模板（如 `{YYYY}/{MM}/{DD}`）
- 支持照片 EXIF 时间与视频元数据时间（MP4/MOV 等）
- 支持时间来源策略：`auto/exif/created/modified`
- 支持复制模式、移动模式、`dry-run` 预演模式
- GUI 支持亮色/深色主题适配、中文字体、结构化日志

## 支持的常见格式

- 图片：`jpg/jpeg/png/heic/heif/gif/tif/tiff/bmp/webp/raw/cr2/cr3/nef/nrw/arw/dng`
- 视频：`mp4/mov/m4v/avi/mkv/mts/3gp`

## 环境要求

- Rust（建议稳定版）
- macOS / Windows / Linux

## 编译

```bash
cargo build --release
```

可执行文件位置：

- `target/release/FileCopyer`

## 使用方式

### 1) 图形界面（GUI）

不传 `--source` / `--target` 时会默认启动 GUI：

```bash
cargo run --release
```

或显式指定：

```bash
cargo run --release -- --gui
```

### 2) 命令行（CLI）

先预演：

```bash
cargo run --release -- \
  --source /Volumes/SDCARD/DCIM \
  --target /Users/you/Pictures/Archive \
  --dry-run
```

实际复制：

```bash
cargo run --release -- \
  --source /Volumes/SDCARD/DCIM \
  --target /Users/you/Pictures/Archive
```

实际移动：

```bash
cargo run --release -- \
  --source /Volumes/SDCARD/DCIM \
  --target /Users/you/Pictures/Archive \
  --mv
```

按自定义层级模板：

```bash
cargo run --release -- \
  --source /Volumes/SDCARD/DCIM \
  --target /Users/you/Pictures/Archive \
  --template "{YYYY}/{MM}/{DD}"
```

## 关键参数

- `--source <PATH>`：源目录（如 SD 卡目录）
- `--target <PATH>`：目标目录
- `--mv`：移动文件（默认复制）
- `--dry-run`：仅输出计划，不写入磁盘
- `--time-source <auto|exif|created|modified>`：日期来源策略
- `--template <TEMPLATE>`：目录模板（支持 `{YYYY}` `{MM}` `{DD}`）
- `--gui`：强制启动 GUI

## 日期来源说明

- `auto`：优先元数据（EXIF/视频），失败时回退到文件时间
- `exif`：仅使用元数据时间
- `created`：仅使用文件创建时间
- `modified`：仅使用文件修改时间

## 注意事项

- 建议首次使用先执行 `--dry-run`，确认归档结果
- 使用 `--mv` 时建议先备份重要数据
- 若目标目录存在同名文件，会自动追加后缀避免覆盖

## 许可证

当前仓库未声明许可证，如需开源发布建议补充 `LICENSE` 文件。
