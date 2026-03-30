//! obsws の永続 state file の読み書きを行うモジュール。
//!
//! state file は obsws の streamServiceSettings と recordDirectory を
//! 再起動後も復元するための JSONC ファイルである。

use std::path::{Path, PathBuf};

use crate::obsws::input_registry::{ObswsInputRegistry, ObswsStreamServiceSettings};

/// state file のトップレベル構造
pub struct ObswsStateFile {
    pub stream: Option<ObswsStateFileStream>,
    pub record: Option<ObswsStateFileRecord>,
}

/// state file の stream セクション
pub struct ObswsStateFileStream {
    pub stream_service_type: String,
    pub server: Option<String>,
    pub key: Option<String>,
}

/// state file の record セクション
pub struct ObswsStateFileRecord {
    pub record_directory: PathBuf,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ObswsStateFile {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let version: i64 = value.to_member("version")?.required()?.try_into()?;
        if version != 1 {
            return Err(value.to_member("version")?.required()?.invalid(format!(
                "unsupported state file version: {version}, expected 1"
            )));
        }
        let stream: Option<ObswsStateFileStream> = value.to_member("stream")?.try_into()?;
        let record: Option<ObswsStateFileRecord> = value.to_member("record")?.try_into()?;
        Ok(Self { stream, record })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ObswsStateFileStream {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let stream_service_type: String = value
            .to_member("streamServiceType")?
            .required()?
            .try_into()?;
        if stream_service_type != "rtmp_custom" {
            return Err(value
                .to_member("streamServiceType")?
                .required()?
                .invalid(format!(
                    "unsupported streamServiceType: \"{stream_service_type}\", expected \"rtmp_custom\""
                )));
        }

        let settings_member = value.to_member("streamServiceSettings")?;
        let settings_value: Option<nojson::RawJsonOwned> = settings_member.try_into()?;
        let (server, key) = if let Some(ref settings) = settings_value {
            let sv = settings.value();
            let server: Option<String> = sv.to_member("server")?.try_into()?;
            let key: Option<String> = sv.to_member("key")?.try_into()?;
            (server, key)
        } else {
            (None, None)
        };

        Ok(Self {
            stream_service_type,
            server,
            key,
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ObswsStateFileRecord {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let record_directory: String =
            value.to_member("recordDirectory")?.required()?.try_into()?;
        if record_directory.is_empty() {
            return Err(value
                .to_member("recordDirectory")?
                .required()?
                .invalid("recordDirectory must not be empty"));
        }
        Ok(Self {
            record_directory: PathBuf::from(record_directory),
        })
    }
}

impl nojson::DisplayJson for ObswsStateFile {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("version", 1)?;
            if let Some(stream) = &self.stream {
                f.member("stream", stream)?;
            }
            if let Some(record) = &self.record {
                f.member("record", record)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for ObswsStateFileStream {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("streamServiceType", &self.stream_service_type)?;
            f.member(
                "streamServiceSettings",
                nojson::object(|f| {
                    if let Some(server) = &self.server {
                        f.member("server", server)?;
                    }
                    if let Some(key) = &self.key {
                        f.member("key", key)?;
                    }
                    Ok(())
                }),
            )
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for ObswsStateFileRecord {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member(
                "recordDirectory",
                self.record_directory.display().to_string(),
            )
        })
        .fmt(f)
    }
}

/// state file を読み込む。
///
/// ファイルが存在しない場合は空の state を返す（初回起動対応）。
/// パースエラーや読み取り権限エラーの場合は起動エラーとする。
pub fn load_state_file(path: &Path) -> crate::Result<ObswsStateFile> {
    if !path.exists() {
        return Ok(ObswsStateFile {
            stream: None,
            record: None,
        });
    }
    crate::json::parse_file(path)
}

/// state file を保存する。
///
/// 一時ファイルへ書き込み後に rename する atomic write を行う。
/// 親ディレクトリが存在しない場合は自動作成する。
// TODO: 将来的にコメント保持更新を検討する
pub fn save_state_file(path: &Path, state: &ObswsStateFile) -> crate::Result<()> {
    let content = crate::json::to_pretty_string(state);

    let dir = path
        .parent()
        .ok_or_else(|| crate::Error::new("state file path has no parent directory"))?;

    // 親ディレクトリが存在しない場合は自動作成する
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| {
            crate::Error::new(format!(
                "failed to create state file directory {}: {e}",
                dir.display()
            ))
        })?;
    }

    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp_path = dir.join(format!(".{file_name}.tmp.{}", std::process::id()));

    std::fs::write(&tmp_path, content.as_bytes()).map_err(|e| {
        crate::Error::new(format!(
            "failed to write temporary state file {}: {e}",
            tmp_path.display()
        ))
    })?;

    std::fs::rename(&tmp_path, path).map_err(|e| {
        // 一時ファイルのクリーンアップを試みる
        let _ = std::fs::remove_file(&tmp_path);
        crate::Error::new(format!(
            "failed to rename state file to {}: {e}",
            path.display()
        ))
    })?;

    Ok(())
}

/// ObswsInputRegistry の現在値から ObswsStateFile を構築する。
pub fn build_state_from_registry(registry: &ObswsInputRegistry) -> ObswsStateFile {
    let settings = registry.stream_service_settings();
    let stream = Some(ObswsStateFileStream {
        stream_service_type: settings.stream_service_type.clone(),
        server: settings.server.clone(),
        key: settings.key.clone(),
    });
    let record = Some(ObswsStateFileRecord {
        record_directory: registry.record_directory().to_path_buf(),
    });
    ObswsStateFile { stream, record }
}

impl ObswsStateFileStream {
    /// ObswsStreamServiceSettings に変換する。
    pub fn to_stream_service_settings(&self) -> ObswsStreamServiceSettings {
        ObswsStreamServiceSettings {
            stream_service_type: self.stream_service_type.clone(),
            server: self.server.clone(),
            key: self.key.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_state_file() {
        let json = r#"{
            "version": 1,
            "stream": {
                "streamServiceType": "rtmp_custom",
                "streamServiceSettings": {
                    "server": "rtmp://127.0.0.1:1935/live",
                    "key": "stream-main"
                }
            },
            "record": {
                "recordDirectory": "/tmp/recordings"
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let stream = state.stream.expect("stream must be present");
        assert_eq!(stream.stream_service_type, "rtmp_custom");
        assert_eq!(stream.server.as_deref(), Some("rtmp://127.0.0.1:1935/live"));
        assert_eq!(stream.key.as_deref(), Some("stream-main"));
        let record = state.record.expect("record must be present");
        assert_eq!(record.record_directory, PathBuf::from("/tmp/recordings"));
    }

    #[test]
    fn parse_stream_only() {
        let json = r#"{
            "version": 1,
            "stream": {
                "streamServiceType": "rtmp_custom",
                "streamServiceSettings": {
                    "server": "rtmp://localhost/live"
                }
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        assert!(state.stream.is_some());
        assert!(state.record.is_none());
    }

    #[test]
    fn parse_record_only() {
        let json = r#"{
            "version": 1,
            "record": {
                "recordDirectory": "/tmp/rec"
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        assert!(state.stream.is_none());
        assert!(state.record.is_some());
    }

    #[test]
    fn parse_empty_state() {
        let json = r#"{ "version": 1 }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        assert!(state.stream.is_none());
        assert!(state.record.is_none());
    }

    #[test]
    fn reject_unsupported_version() {
        let json = r#"{ "version": 2 }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reject_unsupported_stream_service_type() {
        let json = r#"{
            "version": 1,
            "stream": {
                "streamServiceType": "srt_custom",
                "streamServiceSettings": {}
            }
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reject_record_without_record_directory() {
        let json = r#"{
            "version": 1,
            "record": {}
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reject_record_with_empty_record_directory() {
        let json = r#"{
            "version": 1,
            "record": {
                "recordDirectory": ""
            }
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    #[test]
    fn roundtrip_display_and_parse() {
        let state = ObswsStateFile {
            stream: Some(ObswsStateFileStream {
                stream_service_type: "rtmp_custom".to_owned(),
                server: Some("rtmp://127.0.0.1:1935/live".to_owned()),
                key: Some("my-key".to_owned()),
            }),
            record: Some(ObswsStateFileRecord {
                record_directory: PathBuf::from("/tmp/recordings"),
            }),
        };

        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip parse must succeed");

        let stream = parsed.stream.expect("stream must be present");
        assert_eq!(stream.stream_service_type, "rtmp_custom");
        assert_eq!(stream.server.as_deref(), Some("rtmp://127.0.0.1:1935/live"));
        assert_eq!(stream.key.as_deref(), Some("my-key"));
        let record = parsed.record.expect("record must be present");
        assert_eq!(record.record_directory, PathBuf::from("/tmp/recordings"));
    }

    #[test]
    fn save_and_load_state_file() {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let path = dir.path().join("state.jsonc");

        let state = ObswsStateFile {
            stream: Some(ObswsStateFileStream {
                stream_service_type: "rtmp_custom".to_owned(),
                server: Some("rtmp://localhost/live".to_owned()),
                key: None,
            }),
            record: Some(ObswsStateFileRecord {
                record_directory: PathBuf::from("/tmp/rec"),
            }),
        };

        save_state_file(&path, &state).expect("save must succeed");
        assert!(path.exists());

        let loaded: ObswsStateFile = load_state_file(&path).expect("load must succeed");
        let stream = loaded.stream.expect("stream must be present");
        assert_eq!(stream.server.as_deref(), Some("rtmp://localhost/live"));
        assert!(stream.key.is_none());
    }

    #[test]
    fn load_nonexistent_file_returns_empty_state() {
        let path = Path::new("/tmp/nonexistent-hisui-state-file-test.jsonc");
        let state = load_state_file(path).expect("load must succeed for nonexistent file");
        assert!(state.stream.is_none());
        assert!(state.record.is_none());
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let path = dir.path().join("nested").join("dir").join("state.jsonc");

        let state = ObswsStateFile {
            stream: None,
            record: Some(ObswsStateFileRecord {
                record_directory: PathBuf::from("/tmp/rec"),
            }),
        };

        save_state_file(&path, &state).expect("save must succeed");
        assert!(path.exists());
    }

    #[test]
    fn parse_jsonc_with_comments() {
        let json = r#"{
            // state file のバージョン
            "version": 1,
            "stream": {
                "streamServiceType": "rtmp_custom",
                "streamServiceSettings": {
                    "server": "rtmp://127.0.0.1:1935/live"
                    // "key": "secret-key"
                }
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("JSONC parse must succeed");
        let stream = state.stream.expect("stream must be present");
        assert_eq!(stream.server.as_deref(), Some("rtmp://127.0.0.1:1935/live"));
        assert!(stream.key.is_none());
    }
}
