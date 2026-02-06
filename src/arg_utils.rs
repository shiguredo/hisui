use std::path::PathBuf;

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
