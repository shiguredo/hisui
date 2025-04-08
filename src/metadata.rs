use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use orfail::OrFail;

/// Sora の report-*.json から必要な情報のみを取り出した構造体
#[derive(Debug, Clone)]
pub struct RecordingMetadata {
    pub split_only: bool,
    pub archives: Vec<ArchiveEntry>,
}

impl<'text> nojson::FromRawJsonValue<'text> for RecordingMetadata {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        let ([split_only, archives], []) = value.to_fixed_object(["split_only", "archives"], [])?;
        Ok(Self {
            split_only: split_only.try_to()?,
            archives: archives.try_to()?,
        })
    }
}

impl RecordingMetadata {
    pub fn from_file<P: AsRef<Path>>(path: P) -> orfail::Result<Self> {
        let text = std::fs::read_to_string(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        text.parse().map(|nojson::Json(v)| v).or_fail()
    }

    pub fn archive_metadata_paths(&self) -> orfail::Result<Vec<PathBuf>> {
        if self.split_only {
            // split_only の場合には JSON 内に具体的なファイルパスは含まれていないので
            // 命名規則に従って生成する
            let mut paths = Vec::new();
            for archive in &self.archives {
                let last_index_str = archive.split_last_index.as_ref().or_fail()?;
                let last_index = last_index_str.parse::<usize>().or_fail()?;
                for i in 1..=last_index {
                    let path = format!("split-archive-{}_{:04}.json", archive.connection_id, i);
                    paths.push(PathBuf::from(path));
                }
            }
            Ok(paths)
        } else {
            Ok(self
                .archives
                .iter()
                .filter_map(|a| a.metadata_filename.clone())
                .collect())
        }
    }
}

/// Sora の report-*.json の archives 配列の要素に対応する構造体（必要な情報のみ）
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub connection_id: String,
    pub split_last_index: Option<String>,
    pub metadata_filename: Option<PathBuf>,
}

impl<'text> nojson::FromRawJsonValue<'text> for ArchiveEntry {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        let ([connection_id], [split_last_index, metadata_filename]) =
            value.to_fixed_object(["connection_id"], ["split_last_index", "metadata_filename"])?;
        Ok(Self {
            connection_id: connection_id.try_to()?,
            split_last_index: split_last_index.map(|v| v.try_to()).transpose()?,
            metadata_filename: metadata_filename.map(|v| v.try_to()).transpose()?,
        })
    }
}

/// Sora の archive-*.json から必要な情報のみを取り出した構造体
#[derive(Debug, Clone)]
pub struct ArchiveMetadata {
    pub connection_id: String,
    pub format: ContainerFormat,
    pub audio: bool,
    pub video: bool,
    pub start_time_offset: u64,
    pub stop_time_offset: u64,
}

impl<'text> nojson::FromRawJsonValue<'text> for ArchiveMetadata {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        let ([connection_id, format, audio, video, start_time_offset, stop_time_offset], []) =
            value.to_fixed_object(
                [
                    "connection_id",
                    "format",
                    "audio",
                    "video",
                    "start_time_offset",
                    "stop_time_offset",
                ],
                [],
            )?;
        Ok(Self {
            connection_id: connection_id.try_to()?,
            format: format.try_to()?,
            audio: audio.try_to()?,
            video: video.try_to()?,
            start_time_offset: start_time_offset.try_to()?,
            stop_time_offset: stop_time_offset.try_to()?,
        })
    }
}

impl ArchiveMetadata {
    pub fn from_file<P: AsRef<Path>>(path: P) -> orfail::Result<Self> {
        let text = std::fs::read_to_string(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        text.parse().map(|nojson::Json(v)| v).or_fail()
    }

    pub fn source_id(&self) -> SourceId {
        SourceId(Arc::new(self.connection_id.clone()))
    }

    pub fn source_info(&self) -> SourceInfo {
        SourceInfo {
            id: self.source_id(),
            format: self.format,
            start_timestamp: Duration::from_secs(self.start_time_offset),
            stop_timestamp: Duration::from_secs(self.stop_time_offset),
            audio: self.audio,
            video: self.video,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(Arc<String>);

impl nojson::DisplayJson for SourceId {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(&*self.0)
    }
}

impl SourceId {
    pub fn new(id: &str) -> Self {
        Self(Arc::new(id.to_owned()))
    }
}

impl From<SourceId> for String {
    fn from(value: SourceId) -> Self {
        (*value.0).clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    pub id: SourceId,
    pub format: ContainerFormat,
    pub audio: bool,
    pub video: bool,
    pub start_timestamp: Duration,
    pub stop_timestamp: Duration,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ContainerFormat {
    #[default]
    Webm,
    Mp4,
}

impl<'text> nojson::FromRawJsonValue<'text> for ContainerFormat {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        match value.to_unquoted_string_str()?.as_ref() {
            "webm" => Ok(Self::Webm),
            "mp4" => Ok(Self::Mp4),
            v => Err(nojson::JsonParseError::invalid_value(
                value,
                format!("unknown container format: {v}"),
            )),
        }
    }
}
