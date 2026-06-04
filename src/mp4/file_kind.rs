//! MP4 ファイルが通常 MP4 か fragmented MP4 かを判定するモジュール
//!
//! 拡張子ではなくファイルの実体 (ftyp + moov) を読み込んで判定する。
//! `.mp4` 拡張子に fMP4 が格納されていることが普通であるため、実体で判定する。

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use shiguredo_mp4::demux::{Input, Mp4FileKind, Mp4FileKindDetector, RequiredInput};

use crate::{Error, Result};

/// 壊れたファイル対策の読み込みサイズ上限 (100 MB)
///
/// ftyp / moov / 各セグメントを required input として 1 回で読み込む際のサイズ上限。
/// 典型的には 100 MB あれば数百 GB 規模の MP4 ファイルでも扱えるため、実用上の問題はない想定。
const MAX_READ_SIZE: usize = 100 * 1024 * 1024;

/// ファイル先頭 (ftyp + moov) を incremental に読み込んで MP4 / fMP4 を判定する
pub(crate) fn detect_mp4_file_kind<P: AsRef<Path>>(path: P) -> Result<Mp4FileKind> {
    let path = path.as_ref();
    let mut file = File::open(path)
        .map_err(|e| Error::new(format!("Cannot open file {}: {e}", path.display())))?;
    let file_size = file
        .metadata()
        .map_err(|e| Error::new(format!("Cannot stat file {}: {e}", path.display())))?
        .len();

    let mut detector = Mp4FileKindDetector::new();
    while let Some(required) = detector.required_input() {
        let position = required.position;
        let buf = read_required_range(&mut file, file_size, path, required)?;
        detector.handle_input(Input {
            position,
            data: &buf,
        });

        match detector.file_kind() {
            Ok(Some(kind)) => return Ok(kind),
            Ok(None) => {}
            Err(e) => {
                return Err(Error::new(format!(
                    "Cannot detect MP4 file kind {}: {e}",
                    path.display()
                )));
            }
        }
    }

    // required_input が尽きても判定できなかった場合 (moov が見つからない等)
    Err(Error::new(format!(
        "Cannot detect MP4 file kind (moov not found): {}",
        path.display()
    )))
}

/// `RequiredInput` が示す範囲をファイルから読み込む
///
/// `size` が `None` の場合はファイル末尾までを読み込む。
/// 壊れたファイル対策として、読み込みサイズに上限を設ける。
pub(crate) fn read_required_range(
    file: &mut File,
    file_size: u64,
    path: &Path,
    required: RequiredInput,
) -> Result<Vec<u8>> {
    let start = required.position;
    let end = match required.size {
        Some(size) => {
            if size > MAX_READ_SIZE {
                return Err(Error::new(format!(
                    "MP4 file contains box larger than maximum allowed size ({size} > {MAX_READ_SIZE}): {}",
                    path.display()
                )));
            }
            start.saturating_add(size as u64).min(file_size)
        }
        None => file_size,
    };
    let len = usize::try_from(end.saturating_sub(start)).unwrap_or(0);
    if len > MAX_READ_SIZE {
        return Err(Error::new(format!(
            "MP4 file requires reading more than maximum allowed size ({len} > {MAX_READ_SIZE}): {}",
            path.display()
        )));
    }

    let mut buf = vec![0; len];
    file.seek(SeekFrom::Start(start))
        .map_err(|e| Error::new(format!("Seek error {}: {e}", path.display())))?;
    file.read_exact(&mut buf)
        .map_err(|e| Error::new(format!("Read error {}: {e}", path.display())))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_mp4::demux::Mp4FileKind;

    #[test]
    fn detect_regular_mp4() {
        assert_eq!(
            detect_mp4_file_kind("testdata/red-320x320-h264-aac.mp4")
                .expect("通常 MP4 の判定に成功すること"),
            Mp4FileKind::Mp4
        );
    }

    #[test]
    fn detect_fragmented_mp4() {
        assert_eq!(
            detect_mp4_file_kind("testdata/red-320x320-h264-aac-fragmented.mp4")
                .expect("fMP4 の判定に成功すること"),
            Mp4FileKind::FragmentedMp4
        );
    }

    #[test]
    fn detect_rejects_invalid_binary() {
        use std::io::Write;

        let mut file = tempfile::NamedTempFile::new().expect("一時ファイルを作成できること");
        file.write_all(b"this is definitely not a valid mp4 file")
            .expect("一時ファイルに書き込めること");
        let result = detect_mp4_file_kind(file.path());
        assert!(result.is_err(), "不正なバイナリは判定エラーになること");
    }
}
