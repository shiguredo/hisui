use std::path::PathBuf;

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
