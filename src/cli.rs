use std::path::PathBuf;

use clap::Parser;

use crate::types::TimeSource;

#[derive(Parser, Debug)]
#[command(author, version, about = "按日期自动分类照片/视频到 YYYY-MM-DD 文件夹")]
pub struct Args {
    #[arg(short, long, help = "源目录（例如 SD 卡挂载目录）")]
    pub source: Option<PathBuf>,

    #[arg(short, long, help = "目标目录（会自动创建 YYYY-MM-DD 子目录）")]
    pub target: Option<PathBuf>,

    #[arg(
        short,
        long,
        default_value_t = false,
        help = "实际移动文件（默认是复制）"
    )]
    pub mv: bool,

    #[arg(short, long, default_value_t = false, help = "仅打印计划，不执行写入")]
    pub dry_run: bool,

    #[arg(
        long,
        value_enum,
        default_value_t = TimeSource::Auto,
        help = "日期来源策略：auto/exif/created/modified"
    )]
    pub time_source: TimeSource,

    #[arg(
        long,
        default_value = "{YYYY}-{MM}-{DD}",
        help = "目标目录模板，支持 {YYYY} {MM} {DD}（可包含子目录）"
    )]
    pub template: String,

    #[arg(long, default_value_t = false, help = "强制启动图形界面")]
    pub gui: bool,
}
