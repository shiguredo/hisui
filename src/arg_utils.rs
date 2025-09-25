use std::{path::PathBuf, time::Duration};

use indicatif::{ProgressBar, ProgressStyle};

pub fn parse_non_default_opt<T>(opt: noargs::Opt) -> Result<Option<T>, T::Err>
where
    T: std::str::FromStr,
{
    if matches!(opt, noargs::Opt::Default { .. }) {
        Ok(None)
    } else {
        opt.value().parse().map(Some)
    }
}

pub fn validate_existing_directory_path(
    arg: noargs::Arg,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path: PathBuf = arg.value().parse()?;

    if matches!(arg, noargs::Arg::Example { .. }) {
        // ここに来るのは --help によるヘルプ表示の時なのでチェックは不要
    } else if !path.exists() {
        return Err("no such directory".into());
    } else if !path.is_dir() {
        return Err("not a directory".into());
    }

    Ok(path)
}

/// 時間ベースのプログレスバーを作成する
pub fn create_time_progress_bar(total_duration: Duration) -> ProgressBar {
    create_progress_bar(
        total_duration.as_secs(),
        "{spinner:.green} [{elapsed_precise} (ETA: {eta})] [{bar:40.cyan/blue}] {percent}% of {len}s total output duration",
    )
}

/// フレームベースのプログレスバーを作成する
pub fn create_frame_progress_bar(total_frames: u64) -> ProgressBar {
    create_progress_bar(
        total_frames,
        "{spinner:.green} [{elapsed_precise} (ETA: {eta})] [{bar:40.cyan/blue}] {percent}% of {len} total frames",
    )
}

fn create_progress_bar(total: u64, template: &str) -> ProgressBar {
    let progress_bar = ProgressBar::new(total);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(template)
            .unwrap()
            .progress_chars("#>-"),
    );
    progress_bar
}
