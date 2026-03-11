mod cli;
mod gui;
mod media_time;
mod processor;
mod theme;
mod types;

use anyhow::Result;
use clap::Parser;

use crate::cli::Args;
use crate::processor::{format_log_line, process_files};
use crate::types::RunOptions;

fn main() -> Result<()> {
    let args = Args::parse();

    if args.gui || args.source.is_none() || args.target.is_none() {
        return gui::launch_gui();
    }

    let opts = RunOptions {
        source: args.source.expect("source checked"),
        target: args.target.expect("target checked"),
        mv: args.mv,
        dry_run: args.dry_run,
        time_source: args.time_source,
        template: args.template,
    };

    let summary = process_files(&opts, |entry| println!("{}", format_log_line(&entry)))?;
    println!(
        "\n完成: 扫描 {} 个文件，处理 {} 个媒体文件，跳过 {} 个非媒体文件，失败 {} 个文件。",
        summary.scanned, summary.handled, summary.skipped, summary.failed
    );
    Ok(())
}
