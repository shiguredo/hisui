use std::collections::BTreeMap;

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
        assert!(
            registry
                .supported_input_kinds()
                .iter()
                .any(|kind| *kind == "ffmpeg_source")
        );
        assert!(
            registry
                .supported_input_kinds()
                .iter()
                .any(|kind| *kind == "image_source")
        );
        assert!(
            registry
                .supported_input_kinds()
                .iter()
                .any(|kind| *kind == "video_capture_device")
        );
    }
}
