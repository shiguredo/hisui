use std::{num::NonZeroUsize, path::PathBuf, time::Duration};

use indicatif::{ProgressBar, ProgressStyle};

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

// TODO: remove first arg
/// 時間ベースのプログレスバーを作成する
pub fn create_time_progress_bar(show_progress_bar: bool, total_duration: Duration) -> ProgressBar {
    create_progress_bar(
        show_progress_bar,
        total_duration.as_secs(),
        Some("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}s ({eta})"),
        None,
    )
}

/// フレームベースのプログレスバーを作成する
pub fn create_frame_progress_bar(show_progress_bar: bool, total_frames: u64) -> ProgressBar {
    create_progress_bar(
        show_progress_bar,
        total_frames,
        Some("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})"),
        None,
    )
}

 fn create_progress_bar(
   show_progress_bar: bool,
    total: u64,
    template: Option<&str>,
    unit: Option<&str>,
) -> ProgressBar {
    let progress_bar = if show_progress_bar {
        ProgressBar::new(total)
    } else {
        ProgressBar::hidden()
    };

    let unit_str = unit.unwrap_or("");
    let default_template = if unit_str.is_empty() {
        "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})"
    } else {
        &format!(
            "{{spinner:.green}} [{{elapsed_precise}}] [{{bar:40.cyan/blue}}] {{pos}}/{{len}}{unit_str} ({{eta}})"
        )
    };

    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(template.unwrap_or(default_template))
            .unwrap()
            .progress_chars("#>-"),
    );
    progress_bar
}

// TODO(atode): remove
#[cfg(target_os = "macos")]
pub fn maybe_limit_cpu_cores(cores: Option<NonZeroUsize>) -> orfail::Result<()> {
    if cores.is_some() {
        log::warn!("`--cpu-cores` option is ignored on MacOS");
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn maybe_limit_cpu_cores(cores: Option<NonZeroUsize>) -> orfail::Result<()> {
    use orfail::OrFail;

    let Some(cores) = cores else {
        // 制限なし
        return Ok(());
    };

    unsafe {
        let mut cpu_set = std::mem::MaybeUninit::zeroed().assume_init();
        libc::CPU_ZERO(&mut cpu_set);

        for i in 0..cores.get() {
            libc::CPU_SET(i, &mut cpu_set);
        }

        let pid = libc::getpid();
        (libc::sched_setaffinity(pid, std::mem::size_of::<libc::cpu_set_t>(), &cpu_set) == 0)
            .or_fail_with(|()| {
                format!(
                    "Failed to set CPU affinity: {}",
                    std::io::Error::last_os_error()
                )
            })?;
    }
    Ok(())
}
