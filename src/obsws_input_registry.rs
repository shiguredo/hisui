use std::collections::BTreeMap;

use crate::obsws_protocol::OBSWS_DEFAULT_SCENE_NAME;

const OBSWS_SUPPORTED_INPUT_KINDS: [&str; 2] = ["image_source", "video_capture_device"];
const OBSWS_MAX_INPUT_ID_FOR_UUID_SUFFIX: u64 = (1 << 48) - 1;

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsInputEntry {
    pub input_uuid: String,
    pub input_name: String,
    pub input: ObswsInput,
}

impl ObswsInputEntry {
    #[cfg(test)]
    pub fn new_for_test(
        input_uuid: impl Into<String>,
        input_name: impl Into<String>,
        input: ObswsInput,
    ) -> Self {
        Self {
            input_uuid: input_uuid.into(),
            input_name: input_name.into(),
            input,
        }
    }
}

impl nojson::DisplayJson for ObswsInputEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("inputName", &self.input_name)?;
            f.member("inputKind", self.input.kind_name())?;
            // 現状の hisui は OBS の *_v2 / *_v3 のようなバージョン付き input kind を
            // 使っていないため、unversionedInputKind は inputKind と同値になる。
            f.member("unversionedInputKind", self.input.kind_name())?;
            f.member("inputUuid", &self.input_uuid)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsInput {
    pub settings: ObswsInputSettings,
}

impl ObswsInput {
    pub fn from_kind_and_settings(
        input_kind: &str,
        input_settings: nojson::RawJsonValue<'_, '_>,
    ) -> Result<Self, ParseInputSettingsError> {
        Ok(Self {
            settings: ObswsInputSettings::from_kind_and_settings(input_kind, input_settings)?,
        })
    }

    pub fn kind_name(&self) -> &'static str {
        self.settings.kind_name()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObswsInputSettings {
    ImageSource(ObswsImageSourceSettings),
    VideoCaptureDevice(ObswsVideoCaptureDeviceSettings),
}

impl ObswsInputSettings {
    pub fn from_kind_and_settings(
        input_kind: &str,
        input_settings: nojson::RawJsonValue<'_, '_>,
    ) -> Result<Self, ParseInputSettingsError> {
        if input_settings.kind() != nojson::JsonValueKind::Object {
            return Err(ParseInputSettingsError::InvalidInputSettings(
                "Invalid inputSettings field: object is required".to_owned(),
            ));
        }

        match input_kind {
            "image_source" => {
                let file = parse_optional_string_setting(input_settings, "file")?;
                Ok(Self::ImageSource(ObswsImageSourceSettings { file }))
            }
            "video_capture_device" => {
                let device_id = parse_optional_string_setting(input_settings, "device_id")?;
                Ok(Self::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                    device_id,
                }))
            }
            _ => Err(ParseInputSettingsError::UnsupportedInputKind),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::ImageSource(_) => "image_source",
            Self::VideoCaptureDevice(_) => "video_capture_device",
        }
    }
}

impl nojson::DisplayJson for ObswsInputSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::ImageSource(settings) => settings.fmt(f),
            Self::VideoCaptureDevice(settings) => settings.fmt(f),
        }
    }
}

fn parse_optional_string_setting(
    settings: nojson::RawJsonValue<'_, '_>,
    key: &str,
) -> Result<Option<String>, ParseInputSettingsError> {
    let Some(value) = settings
        .to_member(key)
        .map_err(|e| {
            ParseInputSettingsError::InvalidInputSettings(format!(
                "Invalid inputSettings field: {e}"
            ))
        })?
        .get()
    else {
        return Ok(None);
    };

    if value.kind() != nojson::JsonValueKind::String {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: string is required"
        )));
    }
    let value: String = value.try_into().map_err(|e| {
        ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: {e}"
        ))
    })?;
    Ok(Some(value))
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsImageSourceSettings {
    // OBS 互換のため、image_source は file 未指定の状態も有効として扱う
    pub file: Option<String>,
}

impl nojson::DisplayJson for ObswsImageSourceSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(file) = &self.file {
                f.member("file", file)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsVideoCaptureDeviceSettings {
    // OBS 互換のため、video_capture_device は device_id 未指定の状態も有効として扱う
    pub device_id: Option<String>,
}

impl nojson::DisplayJson for ObswsVideoCaptureDeviceSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(device_id) = &self.device_id {
                f.member("device_id", device_id)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseInputSettingsError {
    UnsupportedInputKind,
    InvalidInputSettings(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateInputError {
    UnsupportedSceneName,
    InputNameAlreadyExists,
}

#[derive(Debug, Clone, Default)]
pub struct ObswsInputRegistry {
    inputs_by_uuid: BTreeMap<String, ObswsInputEntry>,
    uuids_by_name: BTreeMap<String, String>,
    next_input_id: u64,
}

impl ObswsInputRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list_inputs(&self) -> Vec<ObswsInputEntry> {
        self.inputs_by_uuid.values().cloned().collect()
    }

    pub fn supported_input_kinds(&self) -> &'static [&'static str] {
        &OBSWS_SUPPORTED_INPUT_KINDS
    }

    pub fn create_input(
        &mut self,
        scene_name: &str,
        input_name: &str,
        input: ObswsInput,
    ) -> Result<ObswsInputEntry, CreateInputError> {
        if scene_name != OBSWS_DEFAULT_SCENE_NAME {
            return Err(CreateInputError::UnsupportedSceneName);
        }
        if self.uuids_by_name.contains_key(input_name) {
            return Err(CreateInputError::InputNameAlreadyExists);
        }

        let input_uuid = self.next_input_uuid();
        let entry = ObswsInputEntry {
            input_uuid: input_uuid.clone(),
            input_name: input_name.to_owned(),
            input,
        };
        self.uuids_by_name
            .insert(entry.input_name.clone(), input_uuid);
        self.inputs_by_uuid
            .insert(entry.input_uuid.clone(), entry.clone());

        Ok(entry)
    }

    pub fn remove_input(
        &mut self,
        input_uuid: Option<&str>,
        input_name: Option<&str>,
    ) -> Option<ObswsInputEntry> {
        if let Some(input_uuid) = input_uuid {
            let removed = self.inputs_by_uuid.remove(input_uuid)?;
            self.uuids_by_name.remove(&removed.input_name);
            return Some(removed);
        }

        let input_name = input_name?;
        let input_uuid = self.uuids_by_name.remove(input_name)?;
        self.inputs_by_uuid.remove(&input_uuid)
    }

    pub fn find_input(
        &self,
        input_uuid: Option<&str>,
        input_name: Option<&str>,
    ) -> Option<&ObswsInputEntry> {
        if let Some(input_uuid) = input_uuid {
            return self.inputs_by_uuid.get(input_uuid);
        }
        let input_name = input_name?;
        let input_uuid = self.uuids_by_name.get(input_name)?;
        self.inputs_by_uuid.get(input_uuid)
    }

    #[cfg(test)]
    pub fn insert_for_test(&mut self, entry: ObswsInputEntry) {
        self.uuids_by_name
            .insert(entry.input_name.clone(), entry.input_uuid.clone());
        self.inputs_by_uuid.insert(entry.input_uuid.clone(), entry);
    }

    fn next_input_uuid(&mut self) -> String {
        let input_id = self.next_input_id;
        if input_id > OBSWS_MAX_INPUT_ID_FOR_UUID_SUFFIX {
            panic!("BUG: obsws input id exceeds 48-bit UUID suffix range");
        }
        self.next_input_id = self
            .next_input_id
            .checked_add(1)
            .expect("BUG: obsws input id overflow");
        format!("00000000-0000-0000-0000-{input_id:012x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_owned_json(text: &str) -> nojson::RawJsonOwned {
        nojson::RawJsonOwned::parse(text).expect("test json must be valid")
    }

    fn empty_video_capture_device_input() -> ObswsInput {
        ObswsInput {
            settings: ObswsInputSettings::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                device_id: None,
            }),
        }
    }

    #[test]
    fn find_input_by_uuid_and_name() {
        let mut registry = ObswsInputRegistry::new();
        registry.insert_for_test(ObswsInputEntry::new_for_test(
            "input-uuid-1",
            "camera-1",
            empty_video_capture_device_input(),
        ));

        let by_uuid = registry.find_input(Some("input-uuid-1"), None);
        assert!(by_uuid.is_some());
        assert_eq!(
            by_uuid.expect("input must exist").input_name,
            "camera-1".to_owned()
        );

        let by_name = registry.find_input(None, Some("camera-1"));
        assert!(by_name.is_some());
        assert_eq!(
            by_name.expect("input must exist").input_uuid,
            "input-uuid-1".to_owned()
        );
    }

    #[test]
    fn supported_input_kinds_contains_expected_values() {
        let registry = ObswsInputRegistry::new();
        assert!(registry.supported_input_kinds().contains(&"image_source"));
        assert!(
            registry
                .supported_input_kinds()
                .contains(&"video_capture_device")
        );
    }

    #[test]
    fn parse_input_settings_rejects_unsupported_kind() {
        let settings = parse_owned_json("{}");
        let error = ObswsInput::from_kind_and_settings("unsupported_kind", settings.value())
            .expect_err("unsupported input kind must be rejected");
        assert_eq!(error, ParseInputSettingsError::UnsupportedInputKind);
    }

    #[test]
    fn parse_input_settings_rejects_non_object() {
        let settings = parse_owned_json("null");
        let error = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect_err("non object settings must be rejected");
        assert_eq!(
            error,
            ParseInputSettingsError::InvalidInputSettings(
                "Invalid inputSettings field: object is required".to_owned()
            )
        );
    }

    #[test]
    fn parse_image_source_settings_reads_file() {
        let settings = parse_owned_json(r#"{"file":"/tmp/image.png"}"#);
        let input = ObswsInput::from_kind_and_settings("image_source", settings.value())
            .expect("image_source settings must be accepted");
        assert_eq!(input.kind_name(), "image_source");
        assert_eq!(
            input.settings,
            ObswsInputSettings::ImageSource(ObswsImageSourceSettings {
                file: Some("/tmp/image.png".to_owned()),
            })
        );
    }

    #[test]
    fn parse_video_capture_device_settings_reads_device_id() {
        let settings = parse_owned_json(r#"{"device_id":"camera-1"}"#);
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("video_capture_device settings must be accepted");
        assert_eq!(input.kind_name(), "video_capture_device");
        assert_eq!(
            input.settings,
            ObswsInputSettings::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                device_id: Some("camera-1".to_owned()),
            })
        );
    }

    #[test]
    fn parse_input_settings_rejects_invalid_known_field_type() {
        let settings = parse_owned_json(r#"{"device_id":1}"#);
        let error = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect_err("invalid known field type must be rejected");
        assert_eq!(
            error,
            ParseInputSettingsError::InvalidInputSettings(
                "Invalid inputSettings.device_id field: string is required".to_owned()
            )
        );
    }

    #[test]
    fn parse_input_settings_ignores_unknown_fields() {
        let settings = parse_owned_json(r#"{"unknown_key":"value"}"#);
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("unknown fields should be ignored");
        let output_json_text = nojson::json(|f| f.value(&input.settings)).to_string();
        let output_json = parse_owned_json(&output_json_text);
        assert!(
            output_json
                .value()
                .to_member("unknown_key")
                .expect("member access must succeed")
                .get()
                .is_none()
        );
    }

    #[test]
    fn create_input_succeeds_with_supported_values() {
        let mut registry = ObswsInputRegistry::new();
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let entry = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input)
            .expect("input creation must succeed");

        assert_eq!(entry.input_name, "camera-1");
        assert_eq!(entry.input.kind_name(), "video_capture_device");
        assert!(registry.find_input(None, Some("camera-1")).is_some());
    }

    #[test]
    fn create_input_rejects_duplicate_name() {
        let mut registry = ObswsInputRegistry::new();
        let first_settings = parse_owned_json("{}");
        let first_input =
            ObswsInput::from_kind_and_settings("video_capture_device", first_settings.value())
                .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", first_input)
            .expect("first input creation must succeed");

        let second_settings = parse_owned_json("{}");
        let second_input =
            ObswsInput::from_kind_and_settings("video_capture_device", second_settings.value())
                .expect("input settings must be valid");
        let error = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", second_input)
            .expect_err("duplicate input name must be rejected");
        assert_eq!(error, CreateInputError::InputNameAlreadyExists);
    }

    #[test]
    fn create_input_rejects_unsupported_scene_name() {
        let mut registry = ObswsInputRegistry::new();
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let error = registry
            .create_input("not-scene", "camera-1", input)
            .expect_err("unsupported scene name must be rejected");
        assert_eq!(error, CreateInputError::UnsupportedSceneName);
    }

    #[test]
    fn remove_input_by_name_succeeds() {
        let mut registry = ObswsInputRegistry::new();
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let created = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input)
            .expect("input creation must succeed");

        let removed = registry.remove_input(None, Some("camera-1"));
        assert!(removed.is_some());
        assert_eq!(
            removed.expect("removed input must exist").input_uuid,
            created.input_uuid
        );
        assert!(registry.find_input(None, Some("camera-1")).is_none());
    }

    #[test]
    fn remove_input_by_uuid_succeeds() {
        let mut registry = ObswsInputRegistry::new();
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let created = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input)
            .expect("input creation must succeed");

        let removed = registry.remove_input(Some(&created.input_uuid), None);
        assert!(removed.is_some());
        assert!(
            registry
                .find_input(Some(&created.input_uuid), None)
                .is_none()
        );
    }

    #[test]
    fn remove_input_returns_none_when_not_found() {
        let mut registry = ObswsInputRegistry::new();
        let removed = registry.remove_input(None, Some("not-found"));
        assert!(removed.is_none());
    }

    #[test]
    #[should_panic(expected = "BUG: obsws input id exceeds 48-bit UUID suffix range")]
    fn create_input_panics_when_uuid_suffix_range_is_exhausted() {
        let mut registry = ObswsInputRegistry::new();
        registry.next_input_id = OBSWS_MAX_INPUT_ID_FOR_UUID_SUFFIX + 1;
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let _ = registry.create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input);
    }
}
