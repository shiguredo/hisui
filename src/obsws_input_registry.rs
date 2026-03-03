use std::collections::BTreeMap;

use crate::obsws_protocol::OBSWS_DEFAULT_SCENE_NAME;

const OBSWS_SUPPORTED_INPUT_KINDS: [&str; 3] =
    ["ffmpeg_source", "image_source", "video_capture_device"];

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ObswsInputEntry {
    pub(crate) input_uuid: String,
    pub(crate) input_name: String,
    pub(crate) input_kind: String,
    pub(crate) settings: crate::json::JsonValue,
}

impl ObswsInputEntry {
    #[cfg(test)]
    pub(crate) fn new_for_test(
        input_uuid: impl Into<String>,
        input_name: impl Into<String>,
        input_kind: impl Into<String>,
        settings: crate::json::JsonValue,
    ) -> Self {
        Self {
            input_uuid: input_uuid.into(),
            input_name: input_name.into(),
            input_kind: input_kind.into(),
            settings,
        }
    }
}

impl nojson::DisplayJson for ObswsInputEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("inputName", &self.input_name)?;
            f.member("inputKind", &self.input_kind)?;
            f.member("unversionedInputKind", &self.input_kind)?;
            f.member("inputUuid", &self.input_uuid)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObswsInputRegistry {
    inputs_by_uuid: BTreeMap<String, ObswsInputEntry>,
    uuids_by_name: BTreeMap<String, String>,
    next_input_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateInputError {
    UnsupportedSceneName,
    UnsupportedInputKind,
    InputNameAlreadyExists,
}

impl ObswsInputRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn list_inputs(&self) -> Vec<ObswsInputEntry> {
        self.inputs_by_uuid.values().cloned().collect()
    }

    pub(crate) fn supported_input_kinds(&self) -> &'static [&'static str] {
        &OBSWS_SUPPORTED_INPUT_KINDS
    }

    pub(crate) fn create_input(
        &mut self,
        scene_name: &str,
        input_name: &str,
        input_kind: &str,
        settings: crate::json::JsonValue,
    ) -> Result<ObswsInputEntry, CreateInputError> {
        if scene_name != OBSWS_DEFAULT_SCENE_NAME {
            return Err(CreateInputError::UnsupportedSceneName);
        }
        if !self.supported_input_kinds().contains(&input_kind) {
            return Err(CreateInputError::UnsupportedInputKind);
        }
        if self.uuids_by_name.contains_key(input_name) {
            return Err(CreateInputError::InputNameAlreadyExists);
        }

        let input_uuid = self.next_input_uuid();
        let entry = ObswsInputEntry {
            input_uuid: input_uuid.clone(),
            input_name: input_name.to_owned(),
            input_kind: input_kind.to_owned(),
            settings,
        };
        self.uuids_by_name
            .insert(entry.input_name.clone(), input_uuid);
        self.inputs_by_uuid
            .insert(entry.input_uuid.clone(), entry.clone());

        Ok(entry)
    }

    pub(crate) fn remove_input(
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

    pub(crate) fn find_input(
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
    pub(crate) fn insert_for_test(&mut self, entry: ObswsInputEntry) {
        self.uuids_by_name
            .insert(entry.input_name.clone(), entry.input_uuid.clone());
        self.inputs_by_uuid.insert(entry.input_uuid.clone(), entry);
    }

    fn next_input_uuid(&mut self) -> String {
        let input_id = self.next_input_id;
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

    fn empty_settings() -> crate::json::JsonValue {
        crate::json::JsonValue::Object(BTreeMap::new())
    }

    #[test]
    fn find_input_by_uuid_and_name() {
        let mut registry = ObswsInputRegistry::new();
        registry.insert_for_test(ObswsInputEntry::new_for_test(
            "input-uuid-1",
            "camera-1",
            "video_capture_device",
            empty_settings(),
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
        assert!(registry.supported_input_kinds().contains(&"ffmpeg_source"));
        assert!(registry.supported_input_kinds().contains(&"image_source"));
        assert!(
            registry
                .supported_input_kinds()
                .contains(&"video_capture_device")
        );
    }

    #[test]
    fn create_input_succeeds_with_supported_values() {
        let mut registry = ObswsInputRegistry::new();
        let entry = registry
            .create_input(
                OBSWS_DEFAULT_SCENE_NAME,
                "camera-1",
                "video_capture_device",
                empty_settings(),
            )
            .expect("input creation must succeed");

        assert_eq!(entry.input_name, "camera-1");
        assert_eq!(entry.input_kind, "video_capture_device");
        assert!(registry.find_input(None, Some("camera-1")).is_some());
    }

    #[test]
    fn create_input_rejects_duplicate_name() {
        let mut registry = ObswsInputRegistry::new();
        registry
            .create_input(
                OBSWS_DEFAULT_SCENE_NAME,
                "camera-1",
                "video_capture_device",
                empty_settings(),
            )
            .expect("first input creation must succeed");

        let error = registry
            .create_input(
                OBSWS_DEFAULT_SCENE_NAME,
                "camera-1",
                "video_capture_device",
                empty_settings(),
            )
            .expect_err("duplicate input name must be rejected");
        assert_eq!(error, CreateInputError::InputNameAlreadyExists);
    }

    #[test]
    fn create_input_rejects_unsupported_kind() {
        let mut registry = ObswsInputRegistry::new();
        let error = registry
            .create_input(
                OBSWS_DEFAULT_SCENE_NAME,
                "camera-1",
                "unsupported_kind",
                empty_settings(),
            )
            .expect_err("unsupported input kind must be rejected");
        assert_eq!(error, CreateInputError::UnsupportedInputKind);
    }

    #[test]
    fn create_input_rejects_unsupported_scene_name() {
        let mut registry = ObswsInputRegistry::new();
        let error = registry
            .create_input(
                "not-scene",
                "camera-1",
                "video_capture_device",
                empty_settings(),
            )
            .expect_err("unsupported scene name must be rejected");
        assert_eq!(error, CreateInputError::UnsupportedSceneName);
    }

    #[test]
    fn remove_input_by_name_succeeds() {
        let mut registry = ObswsInputRegistry::new();
        let created = registry
            .create_input(
                OBSWS_DEFAULT_SCENE_NAME,
                "camera-1",
                "video_capture_device",
                empty_settings(),
            )
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
        let created = registry
            .create_input(
                OBSWS_DEFAULT_SCENE_NAME,
                "camera-1",
                "video_capture_device",
                empty_settings(),
            )
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
}
