use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::obsws_protocol::OBSWS_DEFAULT_SCENE_NAME;

const OBSWS_SUPPORTED_INPUT_KINDS: [&str; 2] = ["image_source", "video_capture_device"];
const OBSWS_MAX_INPUT_ID_FOR_UUID_SUFFIX: u64 = (1 << 48) - 1;
const OBSWS_MAX_SCENE_ID_FOR_UUID_SUFFIX: u64 = (1 << 48) - 1;
const OBSWS_DEFAULT_STREAM_SERVICE_TYPE: &str = "rtmp_custom";

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

    pub fn overlay_with_settings(
        &self,
        input_settings: nojson::RawJsonValue<'_, '_>,
    ) -> Result<Self, ParseInputSettingsError> {
        if input_settings.kind() != nojson::JsonValueKind::Object {
            return Err(ParseInputSettingsError::InvalidInputSettings(
                "Invalid inputSettings field: object is required".to_owned(),
            ));
        }

        match self {
            Self::ImageSource(existing) => {
                let file = parse_overlay_string_setting(input_settings, "file", &existing.file)?;
                Ok(Self::ImageSource(ObswsImageSourceSettings { file }))
            }
            Self::VideoCaptureDevice(existing) => {
                let device_id =
                    parse_overlay_string_setting(input_settings, "device_id", &existing.device_id)?;
                Ok(Self::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                    device_id,
                }))
            }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsSceneEntry {
    pub scene_index: usize,
    pub scene_name: String,
    pub scene_uuid: String,
}

impl nojson::DisplayJson for ObswsSceneEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("sceneIndex", self.scene_index)?;
            f.member("sceneName", &self.scene_name)?;
            f.member("sceneUuid", &self.scene_uuid)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsStreamServiceSettings {
    pub stream_service_type: String,
    pub server: Option<String>,
    pub key: Option<String>,
}

impl Default for ObswsStreamServiceSettings {
    fn default() -> Self {
        Self {
            stream_service_type: OBSWS_DEFAULT_STREAM_SERVICE_TYPE.to_owned(),
            server: None,
            key: None,
        }
    }
}

impl nojson::DisplayJson for ObswsStreamServiceSettings {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsStreamRun {
    pub source_processor_id: String,
    pub encoder_processor_id: String,
    pub endpoint_processor_id: String,
    pub source_track_id: String,
    pub encoded_track_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsRecordRun {
    pub source_processor_id: String,
    pub encoder_processor_id: String,
    pub writer_processor_id: String,
    pub source_track_id: String,
    pub encoded_track_id: String,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone)]
struct ObswsSceneItemState {
    scene_item_id: i64,
    input_uuid: String,
    enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsSceneItemEntry {
    pub scene_item_id: i64,
    pub source_name: String,
    pub source_uuid: String,
    pub scene_item_enabled: bool,
    pub scene_item_locked: bool,
    pub scene_item_index: i64,
    pub is_group: bool,
}

impl nojson::DisplayJson for ObswsSceneItemEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("sceneItemId", self.scene_item_id)?;
            f.member("sourceName", &self.source_name)?;
            f.member("sourceUuid", &self.source_uuid)?;
            f.member("sceneItemEnabled", self.scene_item_enabled)?;
            f.member("sceneItemLocked", self.scene_item_locked)?;
            f.member("sceneItemIndex", self.scene_item_index)?;
            f.member("isGroup", self.is_group)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsSceneItemRef {
    pub scene_name: String,
    pub scene_uuid: String,
    pub scene_item: ObswsSceneItemEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSceneItemIndexResult {
    pub changed: bool,
    pub scene_items: Vec<ObswsSceneItemIndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsSceneItemIndexEntry {
    pub scene_item_id: i64,
    pub scene_item_index: i64,
}

impl nojson::DisplayJson for ObswsSceneItemIndexEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("sceneItemId", self.scene_item_id)?;
            f.member("sceneItemIndex", self.scene_item_index)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone)]
struct ObswsSceneState {
    scene_uuid: String,
    items: Vec<ObswsSceneItemState>,
}

#[derive(Debug, Clone, Default)]
struct ObswsStreamRuntimeState {
    active: bool,
    started_at: Option<Instant>,
    run: Option<ObswsStreamRun>,
}

#[derive(Debug, Clone, Default)]
struct ObswsRecordRuntimeState {
    active: bool,
    started_at: Option<Instant>,
    run: Option<ObswsRecordRun>,
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
        .optional()
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

fn parse_overlay_string_setting(
    settings: nojson::RawJsonValue<'_, '_>,
    key: &str,
    current: &Option<String>,
) -> Result<Option<String>, ParseInputSettingsError> {
    let Some(value) = settings
        .to_member(key)
        .map_err(|e| {
            ParseInputSettingsError::InvalidInputSettings(format!(
                "Invalid inputSettings field: {e}"
            ))
        })?
        .optional()
    else {
        return Ok(current.clone());
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetInputSettingsError {
    InputNotFound,
    InvalidInputSettings(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateSceneError {
    SceneNameAlreadyExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetCurrentProgramSceneError {
    SceneNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveSceneError {
    SceneNotFound,
    LastSceneNotRemovable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateStreamError {
    AlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateRecordError {
    AlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemIdError {
    SceneNotFound,
    SourceNotFound,
    SearchOffsetUnsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemEnabledError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemListError {
    SceneNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateSceneItemError {
    SceneNotFound,
    SourceNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveSceneItemError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemSourceError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemIndexError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSceneItemIndexError {
    SceneNotFound,
    SceneItemNotFound,
    InvalidSceneItemIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateSceneItemError {
    SourceScene,
    DestinationScene,
    SourceSceneItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSceneItemEnabledError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetSceneItemEnabledResult {
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct ObswsInputRegistry {
    inputs_by_uuid: BTreeMap<String, ObswsInputEntry>,
    uuids_by_name: BTreeMap<String, String>,
    scenes_by_name: BTreeMap<String, ObswsSceneState>,
    scene_order: Vec<String>,
    current_program_scene_name: String,
    next_input_id: u64,
    next_scene_id: u64,
    next_scene_item_id: i64,
    next_stream_run_id: u64,
    next_record_run_id: u64,
    stream_service_settings: ObswsStreamServiceSettings,
    stream_runtime: ObswsStreamRuntimeState,
    record_directory: PathBuf,
    record_runtime: ObswsRecordRuntimeState,
}

impl ObswsInputRegistry {
    pub fn new(record_directory: PathBuf) -> Self {
        let mut scenes_by_name = BTreeMap::new();
        scenes_by_name.insert(
            OBSWS_DEFAULT_SCENE_NAME.to_owned(),
            ObswsSceneState {
                scene_uuid: "10000000-0000-0000-0000-000000000000".to_owned(),
                items: Vec::new(),
            },
        );
        Self {
            inputs_by_uuid: BTreeMap::new(),
            uuids_by_name: BTreeMap::new(),
            scenes_by_name,
            scene_order: vec![OBSWS_DEFAULT_SCENE_NAME.to_owned()],
            current_program_scene_name: OBSWS_DEFAULT_SCENE_NAME.to_owned(),
            next_input_id: 0,
            next_scene_id: 1,
            next_scene_item_id: 1,
            next_stream_run_id: 0,
            next_record_run_id: 0,
            stream_service_settings: ObswsStreamServiceSettings::default(),
            stream_runtime: ObswsStreamRuntimeState::default(),
            record_directory,
            record_runtime: ObswsRecordRuntimeState::default(),
        }
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self::new(PathBuf::from("recordings-for-test"))
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
        scene_item_enabled: bool,
    ) -> Result<ObswsInputEntry, CreateInputError> {
        if !self.scenes_by_name.contains_key(scene_name) {
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
        let scene_item_id = self.next_scene_item_id();
        let scene = self
            .scenes_by_name
            .get_mut(scene_name)
            .expect("BUG: scene must exist after validation");
        scene.items.push(ObswsSceneItemState {
            scene_item_id,
            input_uuid: entry.input_uuid.clone(),
            enabled: scene_item_enabled,
        });

        Ok(entry)
    }

    pub fn create_scene(&mut self, scene_name: &str) -> Result<ObswsSceneEntry, CreateSceneError> {
        if self.scenes_by_name.contains_key(scene_name) {
            return Err(CreateSceneError::SceneNameAlreadyExists);
        }
        let scene_id = self.next_scene_id;
        if scene_id > OBSWS_MAX_SCENE_ID_FOR_UUID_SUFFIX {
            panic!("BUG: obsws scene id exceeds 48-bit UUID suffix range");
        }
        self.next_scene_id = self
            .next_scene_id
            .checked_add(1)
            .expect("BUG: obsws scene id overflow");
        let scene_uuid = format!("10000000-0000-0000-0000-{scene_id:012x}");

        self.scenes_by_name.insert(
            scene_name.to_owned(),
            ObswsSceneState {
                scene_uuid: scene_uuid.clone(),
                items: Vec::new(),
            },
        );
        self.scene_order.push(scene_name.to_owned());
        Ok(ObswsSceneEntry {
            scene_index: self.scene_order.len().saturating_sub(1),
            scene_name: scene_name.to_owned(),
            scene_uuid,
        })
    }

    pub fn remove_scene(&mut self, scene_name: &str) -> Result<ObswsSceneEntry, RemoveSceneError> {
        if !self.scenes_by_name.contains_key(scene_name) {
            return Err(RemoveSceneError::SceneNotFound);
        }
        if self.scene_order.len() <= 1 {
            return Err(RemoveSceneError::LastSceneNotRemovable);
        }

        let Some(scene_index) = self.scene_order.iter().position(|name| name == scene_name) else {
            return Err(RemoveSceneError::SceneNotFound);
        };
        let scene_uuid = self
            .scenes_by_name
            .remove(scene_name)
            .map(|scene| scene.scene_uuid)
            .ok_or(RemoveSceneError::SceneNotFound)?;
        self.scene_order.retain(|name| name != scene_name);

        if self.current_program_scene_name == scene_name {
            let new_scene_name = self
                .scene_order
                .first()
                .expect("infallible: at least one scene remains after scene deletion")
                .clone();
            self.current_program_scene_name = new_scene_name;
        }

        Ok(ObswsSceneEntry {
            scene_index,
            scene_name: scene_name.to_owned(),
            scene_uuid,
        })
    }

    pub fn list_scenes(&self) -> Vec<ObswsSceneEntry> {
        self.scene_order
            .iter()
            .enumerate()
            .filter_map(|(index, scene_name)| {
                self.scenes_by_name
                    .get(scene_name)
                    .map(|scene| ObswsSceneEntry {
                        scene_index: index,
                        scene_name: scene_name.clone(),
                        scene_uuid: scene.scene_uuid.clone(),
                    })
            })
            .collect()
    }

    pub fn current_program_scene(&self) -> Option<ObswsSceneEntry> {
        let scene_name = &self.current_program_scene_name;
        let scene = self.scenes_by_name.get(scene_name)?;
        let scene_index = self
            .scene_order
            .iter()
            .position(|name| name == scene_name)?;
        Some(ObswsSceneEntry {
            scene_index,
            scene_name: scene_name.clone(),
            scene_uuid: scene.scene_uuid.clone(),
        })
    }

    pub fn set_current_program_scene(
        &mut self,
        scene_name: &str,
    ) -> Result<(), SetCurrentProgramSceneError> {
        if !self.scenes_by_name.contains_key(scene_name) {
            return Err(SetCurrentProgramSceneError::SceneNotFound);
        }
        self.current_program_scene_name = scene_name.to_owned();
        Ok(())
    }

    pub fn list_current_program_scene_inputs(&self) -> Vec<ObswsInputEntry> {
        let Some(scene) = self.scenes_by_name.get(&self.current_program_scene_name) else {
            return Vec::new();
        };
        scene
            .items
            .iter()
            .filter(|item| item.enabled)
            .filter_map(|item| self.inputs_by_uuid.get(&item.input_uuid).cloned())
            .collect()
    }

    pub fn resolve_scene_name(
        &self,
        scene_name: Option<&str>,
        scene_uuid: Option<&str>,
    ) -> Option<String> {
        if let Some(scene_name) = scene_name {
            if self.scenes_by_name.contains_key(scene_name) {
                return Some(scene_name.to_owned());
            }
            return None;
        }

        let scene_uuid = scene_uuid?;
        self.scenes_by_name
            .iter()
            .find(|(_, scene)| scene.scene_uuid == scene_uuid)
            .map(|(scene_name, _)| scene_name.clone())
    }

    pub fn get_scene_item_id(
        &self,
        scene_name: &str,
        source_name: &str,
        search_offset: i64,
    ) -> Result<i64, GetSceneItemIdError> {
        if search_offset != 0 {
            return Err(GetSceneItemIdError::SearchOffsetUnsupported);
        }
        let Some(scene) = self.scenes_by_name.get(scene_name) else {
            return Err(GetSceneItemIdError::SceneNotFound);
        };
        let Some(scene_item_id) = scene.items.iter().find_map(|item| {
            let input = self.inputs_by_uuid.get(&item.input_uuid)?;
            (input.input_name == source_name).then_some(item.scene_item_id)
        }) else {
            return Err(GetSceneItemIdError::SourceNotFound);
        };
        Ok(scene_item_id)
    }

    pub fn list_scene_items(
        &self,
        scene_name: &str,
    ) -> Result<Vec<ObswsSceneItemEntry>, GetSceneItemListError> {
        let Some(scene) = self.scenes_by_name.get(scene_name) else {
            return Err(GetSceneItemListError::SceneNotFound);
        };
        Ok(self.build_scene_item_entries(scene))
    }

    pub fn create_scene_item(
        &mut self,
        scene_name: &str,
        source_uuid: Option<&str>,
        source_name: Option<&str>,
        scene_item_enabled: bool,
    ) -> Result<ObswsSceneItemRef, CreateSceneItemError> {
        let input_entry = self
            .find_input(source_uuid, source_name)
            .cloned()
            .ok_or(CreateSceneItemError::SourceNotFound)?;
        let scene_item_id = self.next_scene_item_id();
        let scene = self
            .scenes_by_name
            .get_mut(scene_name)
            .ok_or(CreateSceneItemError::SceneNotFound)?;
        scene.items.push(ObswsSceneItemState {
            scene_item_id,
            input_uuid: input_entry.input_uuid.clone(),
            enabled: scene_item_enabled,
        });
        let scene_item_index = scene.items.len().saturating_sub(1) as i64;
        Ok(ObswsSceneItemRef {
            scene_name: scene_name.to_owned(),
            scene_uuid: scene.scene_uuid.clone(),
            scene_item: ObswsSceneItemEntry {
                scene_item_id,
                source_name: input_entry.input_name,
                source_uuid: input_entry.input_uuid,
                scene_item_enabled,
                scene_item_locked: false,
                scene_item_index,
                is_group: false,
            },
        })
    }

    pub fn remove_scene_item(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<ObswsSceneItemRef, RemoveSceneItemError> {
        let scene = self
            .scenes_by_name
            .get_mut(scene_name)
            .ok_or(RemoveSceneItemError::SceneNotFound)?;
        let Some(position) = scene
            .items
            .iter()
            .position(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(RemoveSceneItemError::SceneItemNotFound);
        };
        let removed = scene.items.remove(position);
        let scene_uuid = scene.scene_uuid.clone();
        let input_entry = self
            .inputs_by_uuid
            .get(&removed.input_uuid)
            .ok_or(RemoveSceneItemError::SceneItemNotFound)?;
        Ok(ObswsSceneItemRef {
            scene_name: scene_name.to_owned(),
            scene_uuid,
            scene_item: ObswsSceneItemEntry {
                scene_item_id: removed.scene_item_id,
                source_name: input_entry.input_name.clone(),
                source_uuid: input_entry.input_uuid.clone(),
                scene_item_enabled: removed.enabled,
                scene_item_locked: false,
                scene_item_index: position as i64,
                is_group: false,
            },
        })
    }

    pub fn duplicate_scene_item(
        &mut self,
        from_scene_name: &str,
        to_scene_name: &str,
        scene_item_id: i64,
    ) -> Result<ObswsSceneItemRef, DuplicateSceneItemError> {
        let (input_uuid, enabled) = {
            let from_scene = self
                .scenes_by_name
                .get(from_scene_name)
                .ok_or(DuplicateSceneItemError::SourceScene)?;
            let Some(from_item) = from_scene
                .items
                .iter()
                .find(|item| item.scene_item_id == scene_item_id)
            else {
                return Err(DuplicateSceneItemError::SourceSceneItem);
            };
            (from_item.input_uuid.clone(), from_item.enabled)
        };
        let input_entry = self
            .inputs_by_uuid
            .get(&input_uuid)
            .ok_or(DuplicateSceneItemError::SourceSceneItem)?
            .clone();
        let new_scene_item_id = self.next_scene_item_id();
        let to_scene = self
            .scenes_by_name
            .get_mut(to_scene_name)
            .ok_or(DuplicateSceneItemError::DestinationScene)?;
        to_scene.items.push(ObswsSceneItemState {
            scene_item_id: new_scene_item_id,
            input_uuid: input_uuid.clone(),
            enabled,
        });
        let scene_item_index = to_scene.items.len().saturating_sub(1) as i64;
        Ok(ObswsSceneItemRef {
            scene_name: to_scene_name.to_owned(),
            scene_uuid: to_scene.scene_uuid.clone(),
            scene_item: ObswsSceneItemEntry {
                scene_item_id: new_scene_item_id,
                source_name: input_entry.input_name,
                source_uuid: input_entry.input_uuid,
                scene_item_enabled: enabled,
                scene_item_locked: false,
                scene_item_index,
                is_group: false,
            },
        })
    }

    pub fn get_scene_item_source(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<(String, String), GetSceneItemSourceError> {
        let scene_item = self
            .find_scene_item(scene_name, scene_item_id)
            .map_err(|error| match error {
                FindSceneItemError::SceneNotFound => GetSceneItemSourceError::SceneNotFound,
                FindSceneItemError::SceneItemNotFound => GetSceneItemSourceError::SceneItemNotFound,
            })?;
        let input_entry = self
            .inputs_by_uuid
            .get(&scene_item.input_uuid)
            .ok_or(GetSceneItemSourceError::SceneItemNotFound)?;
        Ok((
            input_entry.input_name.clone(),
            input_entry.input_uuid.clone(),
        ))
    }

    pub fn get_scene_item_index(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<i64, GetSceneItemIndexError> {
        let scene = self
            .scenes_by_name
            .get(scene_name)
            .ok_or(GetSceneItemIndexError::SceneNotFound)?;
        let Some(index) = scene
            .items
            .iter()
            .position(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(GetSceneItemIndexError::SceneItemNotFound);
        };
        Ok(index as i64)
    }

    pub fn set_scene_item_index(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
        scene_item_index: i64,
    ) -> Result<SetSceneItemIndexResult, SetSceneItemIndexError> {
        let scene = self
            .scenes_by_name
            .get_mut(scene_name)
            .ok_or(SetSceneItemIndexError::SceneNotFound)?;
        if scene_item_index < 0 || scene_item_index as usize >= scene.items.len() {
            return Err(SetSceneItemIndexError::InvalidSceneItemIndex);
        }
        let Some(current_index) = scene
            .items
            .iter()
            .position(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(SetSceneItemIndexError::SceneItemNotFound);
        };
        let target_index = scene_item_index as usize;
        if current_index != target_index {
            let moved = scene.items.remove(current_index);
            scene.items.insert(target_index, moved);
        }
        Ok(SetSceneItemIndexResult {
            changed: current_index != target_index,
            scene_items: scene
                .items
                .iter()
                .enumerate()
                .map(|(index, item)| ObswsSceneItemIndexEntry {
                    scene_item_id: item.scene_item_id,
                    scene_item_index: index as i64,
                })
                .collect(),
        })
    }

    pub fn set_scene_item_enabled(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
        enabled: bool,
    ) -> Result<SetSceneItemEnabledResult, SetSceneItemEnabledError> {
        let Some(scene) = self.scenes_by_name.get_mut(scene_name) else {
            return Err(SetSceneItemEnabledError::SceneNotFound);
        };
        let Some(scene_item) = scene
            .items
            .iter_mut()
            .find(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(SetSceneItemEnabledError::SceneItemNotFound);
        };
        let changed = scene_item.enabled != enabled;
        scene_item.enabled = enabled;
        Ok(SetSceneItemEnabledResult { changed })
    }

    pub fn get_scene_item_enabled(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<bool, GetSceneItemEnabledError> {
        let Some(scene) = self.scenes_by_name.get(scene_name) else {
            return Err(GetSceneItemEnabledError::SceneNotFound);
        };
        let Some(scene_item) = scene
            .items
            .iter()
            .find(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(GetSceneItemEnabledError::SceneItemNotFound);
        };
        Ok(scene_item.enabled)
    }

    pub fn stream_service_settings(&self) -> &ObswsStreamServiceSettings {
        &self.stream_service_settings
    }

    pub fn next_stream_run_id(&mut self) -> u64 {
        let run_id = self.next_stream_run_id;
        self.next_stream_run_id = self
            .next_stream_run_id
            .checked_add(1)
            .expect("BUG: obsws stream run id overflow");
        run_id
    }

    pub fn next_record_run_id(&mut self) -> u64 {
        let run_id = self.next_record_run_id;
        self.next_record_run_id = self
            .next_record_run_id
            .checked_add(1)
            .expect("BUG: obsws record run id overflow");
        run_id
    }

    pub fn set_stream_service_settings(&mut self, settings: ObswsStreamServiceSettings) {
        self.stream_service_settings = settings;
    }

    pub fn activate_stream(&mut self, run: ObswsStreamRun) -> Result<(), ActivateStreamError> {
        if self.stream_runtime.active {
            return Err(ActivateStreamError::AlreadyActive);
        }
        self.stream_runtime.active = true;
        self.stream_runtime.started_at = Some(Instant::now());
        self.stream_runtime.run = Some(run);
        Ok(())
    }

    pub fn deactivate_stream(&mut self) -> Option<ObswsStreamRun> {
        let run = self.stream_runtime.run.take();
        self.stream_runtime.active = false;
        self.stream_runtime.started_at = None;
        run
    }

    pub fn is_stream_active(&self) -> bool {
        self.stream_runtime.active
    }

    pub fn stream_run(&self) -> Option<ObswsStreamRun> {
        self.stream_runtime.run.clone()
    }

    pub fn stream_uptime(&self) -> Duration {
        self.stream_runtime
            .started_at
            .map(|started_at| started_at.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    pub fn record_directory(&self) -> &Path {
        &self.record_directory
    }

    pub fn set_record_directory(&mut self, record_directory: PathBuf) {
        self.record_directory = record_directory;
    }

    pub fn activate_record(&mut self, run: ObswsRecordRun) -> Result<(), ActivateRecordError> {
        if self.record_runtime.active {
            return Err(ActivateRecordError::AlreadyActive);
        }
        self.record_runtime.active = true;
        self.record_runtime.started_at = Some(Instant::now());
        self.record_runtime.run = Some(run);
        Ok(())
    }

    pub fn deactivate_record(&mut self) -> Option<ObswsRecordRun> {
        let run = self.record_runtime.run.take();
        self.record_runtime.active = false;
        self.record_runtime.started_at = None;
        run
    }

    pub fn is_record_active(&self) -> bool {
        self.record_runtime.active
    }

    pub fn record_run(&self) -> Option<ObswsRecordRun> {
        self.record_runtime.run.clone()
    }

    pub fn record_uptime(&self) -> Duration {
        self.record_runtime
            .started_at
            .map(|started_at| started_at.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    pub fn record_output_path(&self) -> Option<&Path> {
        self.record_runtime
            .run
            .as_ref()
            .map(|run| run.output_path.as_path())
    }

    pub fn remove_input(
        &mut self,
        input_uuid: Option<&str>,
        input_name: Option<&str>,
    ) -> Option<ObswsInputEntry> {
        if let Some(input_uuid) = input_uuid {
            let removed = self.inputs_by_uuid.remove(input_uuid)?;
            self.uuids_by_name.remove(&removed.input_name);
            for scene in self.scenes_by_name.values_mut() {
                scene
                    .items
                    .retain(|item| item.input_uuid != removed.input_uuid);
            }
            return Some(removed);
        }

        let input_name = input_name?;
        let input_uuid = self.uuids_by_name.remove(input_name)?;
        let removed = self.inputs_by_uuid.remove(&input_uuid)?;
        for scene in self.scenes_by_name.values_mut() {
            scene
                .items
                .retain(|item| item.input_uuid != removed.input_uuid);
        }
        Some(removed)
    }

    pub fn set_input_settings(
        &mut self,
        input_uuid: Option<&str>,
        input_name: Option<&str>,
        input_settings: nojson::RawJsonValue<'_, '_>,
        overlay: bool,
    ) -> Result<(), SetInputSettingsError> {
        let target_input_uuid = if let Some(input_uuid) = input_uuid {
            input_uuid.to_owned()
        } else {
            let Some(input_name) = input_name else {
                return Err(SetInputSettingsError::InputNotFound);
            };
            let Some(input_uuid) = self.uuids_by_name.get(input_name) else {
                return Err(SetInputSettingsError::InputNotFound);
            };
            input_uuid.clone()
        };

        let Some(input_entry) = self.inputs_by_uuid.get_mut(&target_input_uuid) else {
            return Err(SetInputSettingsError::InputNotFound);
        };

        let settings_result = if overlay {
            input_entry
                .input
                .settings
                .overlay_with_settings(input_settings)
        } else {
            ObswsInputSettings::from_kind_and_settings(
                input_entry.input.kind_name(),
                input_settings,
            )
        };
        let settings = settings_result.map_err(|e| match e {
            ParseInputSettingsError::InvalidInputSettings(message) => {
                SetInputSettingsError::InvalidInputSettings(message)
            }
            ParseInputSettingsError::UnsupportedInputKind => {
                SetInputSettingsError::InvalidInputSettings(
                    "Unsupported input kind for inputSettings update".to_owned(),
                )
            }
        })?;
        input_entry.input.settings = settings;
        Ok(())
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

    fn next_scene_item_id(&mut self) -> i64 {
        let scene_item_id = self.next_scene_item_id;
        self.next_scene_item_id = self
            .next_scene_item_id
            .checked_add(1)
            .expect("BUG: obsws scene item id overflow");
        scene_item_id
    }

    fn build_scene_item_entries(&self, scene: &ObswsSceneState) -> Vec<ObswsSceneItemEntry> {
        scene
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let input_entry = self.inputs_by_uuid.get(&item.input_uuid)?;
                Some(ObswsSceneItemEntry {
                    scene_item_id: item.scene_item_id,
                    source_name: input_entry.input_name.clone(),
                    source_uuid: input_entry.input_uuid.clone(),
                    scene_item_enabled: item.enabled,
                    scene_item_locked: false,
                    scene_item_index: index as i64,
                    is_group: false,
                })
            })
            .collect()
    }

    fn find_scene_item(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<&ObswsSceneItemState, FindSceneItemError> {
        let scene = self
            .scenes_by_name
            .get(scene_name)
            .ok_or(FindSceneItemError::SceneNotFound)?;
        scene
            .items
            .iter()
            .find(|item| item.scene_item_id == scene_item_id)
            .ok_or(FindSceneItemError::SceneItemNotFound)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindSceneItemError {
    SceneNotFound,
    SceneItemNotFound,
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
        let mut registry = ObswsInputRegistry::new_for_test();
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
        let registry = ObswsInputRegistry::new_for_test();
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
                .optional()
                .is_none()
        );
    }

    #[test]
    fn create_input_succeeds_with_supported_values() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let entry = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");

        assert_eq!(entry.input_name, "camera-1");
        assert_eq!(entry.input.kind_name(), "video_capture_device");
        assert!(registry.find_input(None, Some("camera-1")).is_some());
    }

    #[test]
    fn create_input_rejects_duplicate_name() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let first_settings = parse_owned_json("{}");
        let first_input =
            ObswsInput::from_kind_and_settings("video_capture_device", first_settings.value())
                .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", first_input, true)
            .expect("first input creation must succeed");

        let second_settings = parse_owned_json("{}");
        let second_input =
            ObswsInput::from_kind_and_settings("video_capture_device", second_settings.value())
                .expect("input settings must be valid");
        let error = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", second_input, true)
            .expect_err("duplicate input name must be rejected");
        assert_eq!(error, CreateInputError::InputNameAlreadyExists);
    }

    #[test]
    fn create_input_rejects_unsupported_scene_name() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let error = registry
            .create_input("not-scene", "camera-1", input, true)
            .expect_err("unsupported scene name must be rejected");
        assert_eq!(error, CreateInputError::UnsupportedSceneName);
    }

    #[test]
    fn set_input_settings_with_overlay_updates_specified_fields_only() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json(r#"{"device_id":"camera-1"}"#).value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");

        registry
            .set_input_settings(
                None,
                Some("camera-1"),
                parse_owned_json(r#"{}"#).value(),
                true,
            )
            .expect("overlay update must succeed");
        let untouched = registry
            .find_input(None, Some("camera-1"))
            .expect("input must exist");
        assert_eq!(
            untouched.input.settings,
            ObswsInputSettings::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                device_id: Some("camera-1".to_owned()),
            })
        );

        registry
            .set_input_settings(
                None,
                Some("camera-1"),
                parse_owned_json(r#"{"device_id":"camera-2"}"#).value(),
                true,
            )
            .expect("overlay update must succeed");
        let updated = registry
            .find_input(None, Some("camera-1"))
            .expect("input must exist");
        assert_eq!(
            updated.input.settings,
            ObswsInputSettings::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                device_id: Some("camera-2".to_owned()),
            })
        );
    }

    #[test]
    fn set_input_settings_without_overlay_replaces_existing_settings() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json(r#"{"device_id":"camera-1"}"#).value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");

        registry
            .set_input_settings(
                None,
                Some("camera-1"),
                parse_owned_json(r#"{}"#).value(),
                false,
            )
            .expect("replace update must succeed");
        let updated = registry
            .find_input(None, Some("camera-1"))
            .expect("input must exist");
        assert_eq!(
            updated.input.settings,
            ObswsInputSettings::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                device_id: None,
            })
        );
    }

    #[test]
    fn set_input_settings_returns_not_found_error_for_unknown_input() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let error = registry
            .set_input_settings(
                None,
                Some("not-found"),
                parse_owned_json(r#"{}"#).value(),
                true,
            )
            .expect_err("unknown input must be rejected");
        assert_eq!(error, SetInputSettingsError::InputNotFound);
    }

    #[test]
    fn get_scene_item_id_assigns_global_sequential_ids() {
        let mut registry = ObswsInputRegistry::new_for_test();
        registry
            .create_scene("Scene B")
            .expect("scene creation must succeed");

        let input_a = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-a", input_a, true)
            .expect("input creation must succeed");

        let input_b = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene B", "camera-b", input_b, true)
            .expect("input creation must succeed");

        let scene_item_id_a = registry
            .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-a", 0)
            .expect("scene item id must exist");
        let scene_item_id_b = registry
            .get_scene_item_id("Scene B", "camera-b", 0)
            .expect("scene item id must exist");
        assert_eq!(scene_item_id_a, 1);
        assert_eq!(scene_item_id_b, 2);
    }

    #[test]
    fn get_scene_item_id_rejects_non_zero_search_offset() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");

        let error = registry
            .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 1)
            .expect_err("non zero search offset must be rejected");
        assert_eq!(error, GetSceneItemIdError::SearchOffsetUnsupported);
    }

    #[test]
    fn get_scene_item_id_returns_not_found_errors() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");

        let scene_error = registry
            .get_scene_item_id("Scene B", "camera-1", 0)
            .expect_err("unknown scene must be rejected");
        assert_eq!(scene_error, GetSceneItemIdError::SceneNotFound);

        let source_error = registry
            .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-unknown", 0)
            .expect_err("unknown source must be rejected");
        assert_eq!(source_error, GetSceneItemIdError::SourceNotFound);
    }

    #[test]
    fn set_scene_item_enabled_updates_scene_item_state() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");

        assert_eq!(registry.list_current_program_scene_inputs().len(), 1);

        let scene_item_id = registry
            .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 0)
            .expect("scene item id must exist");
        let first_result = registry
            .set_scene_item_enabled(OBSWS_DEFAULT_SCENE_NAME, scene_item_id, false)
            .expect("set scene item enabled must succeed");
        assert!(first_result.changed);
        assert_eq!(registry.list_current_program_scene_inputs().len(), 0);

        let second_result = registry
            .set_scene_item_enabled(OBSWS_DEFAULT_SCENE_NAME, scene_item_id, false)
            .expect("set scene item enabled must succeed");
        assert!(!second_result.changed);
    }

    #[test]
    fn set_scene_item_enabled_returns_not_found_errors() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");

        let scene_error = registry
            .set_scene_item_enabled("Scene B", 1, false)
            .expect_err("unknown scene must be rejected");
        assert_eq!(scene_error, SetSceneItemEnabledError::SceneNotFound);

        let item_error = registry
            .set_scene_item_enabled(OBSWS_DEFAULT_SCENE_NAME, 999, false)
            .expect_err("unknown scene item id must be rejected");
        assert_eq!(item_error, SetSceneItemEnabledError::SceneItemNotFound);
    }

    #[test]
    fn get_scene_item_enabled_returns_current_state() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 0)
            .expect("scene item id must exist");

        let initial_enabled = registry
            .get_scene_item_enabled(OBSWS_DEFAULT_SCENE_NAME, scene_item_id)
            .expect("scene item state must be retrievable");
        assert!(initial_enabled);

        registry
            .set_scene_item_enabled(OBSWS_DEFAULT_SCENE_NAME, scene_item_id, false)
            .expect("set scene item enabled must succeed");

        let updated_enabled = registry
            .get_scene_item_enabled(OBSWS_DEFAULT_SCENE_NAME, scene_item_id)
            .expect("scene item state must be retrievable");
        assert!(!updated_enabled);
    }

    #[test]
    fn create_input_with_scene_item_disabled_creates_disabled_scene_item() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, false)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 0)
            .expect("scene item id must exist");

        let scene_item_enabled = registry
            .get_scene_item_enabled(OBSWS_DEFAULT_SCENE_NAME, scene_item_id)
            .expect("scene item state must be retrievable");
        assert!(!scene_item_enabled);
    }

    #[test]
    fn get_scene_item_enabled_returns_not_found_errors() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 0)
            .expect("scene item id must exist");

        let scene_error = registry
            .get_scene_item_enabled("Scene B", scene_item_id)
            .expect_err("unknown scene must be rejected");
        assert_eq!(scene_error, GetSceneItemEnabledError::SceneNotFound);

        let scene_item_error = registry
            .get_scene_item_enabled(OBSWS_DEFAULT_SCENE_NAME, 999)
            .expect_err("unknown scene item id must be rejected");
        assert_eq!(
            scene_item_error,
            GetSceneItemEnabledError::SceneItemNotFound
        );
    }

    #[test]
    fn create_scene_item_and_list_scene_items_succeed() {
        let mut registry = ObswsInputRegistry::new_for_test();
        registry
            .create_scene("Scene B")
            .expect("scene creation must succeed");
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        let created_input = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, false)
            .expect("input creation must succeed");

        let created_scene_item = registry
            .create_scene_item("Scene B", Some(&created_input.input_uuid), None, true)
            .expect("scene item creation must succeed");
        assert_eq!(created_scene_item.scene_name, "Scene B");

        let scene_items = registry
            .list_scene_items("Scene B")
            .expect("scene items must be listed");
        assert_eq!(scene_items.len(), 1);
        assert_eq!(scene_items[0].source_name, "camera-1");
        assert_eq!(scene_items[0].scene_item_index, 0);
    }

    #[test]
    fn remove_scene_item_and_set_scene_item_index_succeed() {
        let mut registry = ObswsInputRegistry::new_for_test();
        registry
            .create_scene("Scene B")
            .expect("scene creation must succeed");
        let input_1 = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        let created_input_1 = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input_1, false)
            .expect("input creation must succeed");
        let input_2 = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        let created_input_2 = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-2", input_2, false)
            .expect("input creation must succeed");

        let first_scene_item = registry
            .create_scene_item("Scene B", Some(&created_input_1.input_uuid), None, true)
            .expect("scene item creation must succeed");
        let second_scene_item = registry
            .create_scene_item("Scene B", Some(&created_input_2.input_uuid), None, true)
            .expect("scene item creation must succeed");

        let set_index_result = registry
            .set_scene_item_index("Scene B", second_scene_item.scene_item.scene_item_id, 0)
            .expect("set scene item index must succeed");
        assert!(set_index_result.changed);
        assert_eq!(
            set_index_result.scene_items[0].scene_item_id,
            second_scene_item.scene_item.scene_item_id
        );
        assert_eq!(
            set_index_result.scene_items[1].scene_item_id,
            first_scene_item.scene_item.scene_item_id
        );

        let removed_scene_item = registry
            .remove_scene_item("Scene B", first_scene_item.scene_item.scene_item_id)
            .expect("scene item removal must succeed");
        assert_eq!(removed_scene_item.scene_item.source_name, "camera-1");
        let scene_items = registry
            .list_scene_items("Scene B")
            .expect("scene items must be listed");
        assert_eq!(scene_items.len(), 1);
        assert_eq!(scene_items[0].source_name, "camera-2");
    }

    #[test]
    fn duplicate_scene_item_to_another_scene_succeeds() {
        let mut registry = ObswsInputRegistry::new_for_test();
        registry
            .create_scene("Scene B")
            .expect("scene creation must succeed");
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            parse_owned_json("{}").value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 0)
            .expect("scene item id must exist");

        let duplicated = registry
            .duplicate_scene_item(OBSWS_DEFAULT_SCENE_NAME, "Scene B", scene_item_id)
            .expect("scene item duplication must succeed");
        assert!(duplicated.scene_item.scene_item_id > scene_item_id);
        assert_eq!(duplicated.scene_name, "Scene B");
        assert_eq!(duplicated.scene_item.source_name, "camera-1");

        let scene_b_items = registry
            .list_scene_items("Scene B")
            .expect("scene items must be listed");
        assert_eq!(scene_b_items.len(), 1);
        assert_eq!(scene_b_items[0].source_name, "camera-1");
    }

    #[test]
    fn remove_input_by_name_succeeds() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let created = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
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
        let mut registry = ObswsInputRegistry::new_for_test();
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let created = registry
            .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
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
        let mut registry = ObswsInputRegistry::new_for_test();
        let removed = registry.remove_input(None, Some("not-found"));
        assert!(removed.is_none());
    }

    #[test]
    fn scene_list_contains_default_scene() {
        let registry = ObswsInputRegistry::new_for_test();
        let scenes = registry.list_scenes();
        assert_eq!(scenes.len(), 1);
        assert_eq!(scenes[0].scene_name, OBSWS_DEFAULT_SCENE_NAME);
        assert_eq!(
            registry
                .current_program_scene()
                .map(|scene| scene.scene_name),
            Some(OBSWS_DEFAULT_SCENE_NAME.to_owned())
        );
    }

    #[test]
    fn create_scene_and_set_current_program_scene_succeeds() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let created = registry
            .create_scene("Scene B")
            .expect("scene creation must succeed");
        assert_eq!(created.scene_name, "Scene B");

        registry
            .set_current_program_scene("Scene B")
            .expect("setting current scene must succeed");
        assert_eq!(
            registry
                .current_program_scene()
                .map(|scene| scene.scene_name),
            Some("Scene B".to_owned())
        );
    }

    #[test]
    fn remove_scene_removes_non_current_scene() {
        let mut registry = ObswsInputRegistry::new_for_test();
        registry
            .create_scene("Scene B")
            .expect("scene creation must succeed");

        let removed = registry
            .remove_scene("Scene B")
            .expect("scene removal must succeed");
        assert_eq!(removed.scene_name, "Scene B");
        assert_eq!(registry.list_scenes().len(), 1);
        assert_eq!(
            registry
                .current_program_scene()
                .map(|scene| scene.scene_name),
            Some(OBSWS_DEFAULT_SCENE_NAME.to_owned())
        );
    }

    #[test]
    fn remove_scene_switches_current_program_scene_when_current_is_removed() {
        let mut registry = ObswsInputRegistry::new_for_test();
        registry
            .create_scene("Scene B")
            .expect("scene creation must succeed");
        registry
            .set_current_program_scene("Scene B")
            .expect("setting current scene must succeed");

        registry
            .remove_scene("Scene B")
            .expect("scene removal must succeed");
        assert_eq!(
            registry
                .current_program_scene()
                .map(|scene| scene.scene_name),
            Some(OBSWS_DEFAULT_SCENE_NAME.to_owned())
        );
    }

    #[test]
    fn remove_scene_rejects_deleting_last_scene() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let error = registry
            .remove_scene(OBSWS_DEFAULT_SCENE_NAME)
            .expect_err("last scene removal must fail");
        assert_eq!(error, RemoveSceneError::LastSceneNotRemovable);
    }

    #[test]
    fn stream_runtime_state_changes_on_activate_and_deactivate() {
        let mut registry = ObswsInputRegistry::new_for_test();
        assert!(!registry.is_stream_active());
        assert_eq!(registry.stream_uptime(), Duration::ZERO);

        registry
            .activate_stream(ObswsStreamRun {
                source_processor_id: "source".to_owned(),
                encoder_processor_id: "encoder".to_owned(),
                endpoint_processor_id: "endpoint".to_owned(),
                source_track_id: "source-track".to_owned(),
                encoded_track_id: "encoded-track".to_owned(),
            })
            .expect("stream activation must succeed");
        assert!(registry.is_stream_active());

        registry.deactivate_stream();
        assert!(!registry.is_stream_active());
        assert_eq!(registry.stream_uptime(), Duration::ZERO);
    }

    #[test]
    #[should_panic(expected = "BUG: obsws input id exceeds 48-bit UUID suffix range")]
    fn create_input_panics_when_uuid_suffix_range_is_exhausted() {
        let mut registry = ObswsInputRegistry::new_for_test();
        registry.next_input_id = OBSWS_MAX_INPUT_ID_FOR_UUID_SUFFIX + 1;
        let settings = parse_owned_json("{}");
        let input = ObswsInput::from_kind_and_settings("video_capture_device", settings.value())
            .expect("input settings must be valid");
        let _ = registry.create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true);
    }
}
