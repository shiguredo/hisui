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
    assert!(
        registry
            .supported_input_kinds()
            .contains(&"mp4_file_source")
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
fn parse_mp4_file_source_settings_reads_path_and_loop_playback() {
    let settings = parse_owned_json(r#"{"path":"/tmp/input.mp4","loopPlayback":true}"#);
    let input = ObswsInput::from_kind_and_settings("mp4_file_source", settings.value())
        .expect("mp4_file_source settings must be accepted");
    assert_eq!(input.kind_name(), "mp4_file_source");
    assert_eq!(
        input.settings,
        ObswsInputSettings::Mp4FileSource(ObswsMp4FileSourceSettings {
            path: Some("/tmp/input.mp4".to_owned()),
            loop_playback: true,
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
        ObswsInputSettings::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings { device_id: None })
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
fn set_input_name_updates_name_lookup_and_entry() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
            .expect("input settings must be valid");
    registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
        .expect("input creation must succeed");

    registry
        .set_input_name(None, Some("camera-1"), "camera-renamed")
        .expect("input rename must succeed");

    assert!(registry.find_input(None, Some("camera-1")).is_none());
    let renamed = registry
        .find_input(None, Some("camera-renamed"))
        .expect("renamed input must exist");
    assert_eq!(renamed.input_name, "camera-renamed");
}

#[test]
fn set_input_name_returns_expected_errors() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input_1 =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
            .expect("input settings must be valid");
    registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input_1, true)
        .expect("input creation must succeed");
    let input_2 =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
            .expect("input settings must be valid");
    registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-2", input_2, true)
        .expect("input creation must succeed");

    let not_found = registry
        .set_input_name(None, Some("not-found"), "new-name")
        .expect_err("unknown input must be rejected");
    assert_eq!(not_found, SetInputNameError::InputNotFound);

    let duplicated = registry
        .set_input_name(None, Some("camera-1"), "camera-2")
        .expect_err("duplicated input name must be rejected");
    assert_eq!(duplicated, SetInputNameError::InputNameAlreadyExists);
}

#[test]
fn get_input_default_settings_returns_default_object_per_kind() {
    let registry = ObswsInputRegistry::new_for_test();

    let image_default = registry
        .get_input_default_settings("image_source")
        .expect("image_source defaults must be available");
    assert_eq!(
        image_default,
        ObswsInputSettings::ImageSource(ObswsImageSourceSettings::default())
    );

    let device_default = registry
        .get_input_default_settings("video_capture_device")
        .expect("video_capture_device defaults must be available");
    assert_eq!(
        device_default,
        ObswsInputSettings::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings::default())
    );

    let mp4_default = registry
        .get_input_default_settings("mp4_file_source")
        .expect("mp4_file_source defaults must be available");
    assert_eq!(
        mp4_default,
        ObswsInputSettings::Mp4FileSource(ObswsMp4FileSourceSettings::default())
    );
}

#[test]
fn get_input_default_settings_rejects_unsupported_kind() {
    let registry = ObswsInputRegistry::new_for_test();
    let error = registry
        .get_input_default_settings("unsupported_kind")
        .expect_err("unsupported input kind must be rejected");
    assert_eq!(error, ParseInputSettingsError::UnsupportedInputKind);
}

#[test]
fn get_scene_item_id_assigns_global_sequential_ids() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");

    let input_a =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
            .expect("input settings must be valid");
    registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-a", input_a, true)
        .expect("input creation must succeed");

    let input_b =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
fn set_and_get_scene_item_locked_succeeds() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
            .expect("input settings must be valid");
    registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 0)
        .expect("scene item id must exist");

    let initial_locked = registry
        .get_scene_item_locked(OBSWS_DEFAULT_SCENE_NAME, scene_item_id)
        .expect("scene item lock state must be retrievable");
    assert!(!initial_locked);

    let set_result = registry
        .set_scene_item_locked(OBSWS_DEFAULT_SCENE_NAME, scene_item_id, true)
        .expect("set scene item locked must succeed");
    assert!(set_result.changed);

    let updated_locked = registry
        .get_scene_item_locked(OBSWS_DEFAULT_SCENE_NAME, scene_item_id)
        .expect("scene item lock state must be retrievable");
    assert!(updated_locked);
}

#[test]
fn set_and_get_scene_item_blend_mode_succeeds() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
            .expect("input settings must be valid");
    registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 0)
        .expect("scene item id must exist");

    let initial_blend_mode = registry
        .get_scene_item_blend_mode(OBSWS_DEFAULT_SCENE_NAME, scene_item_id)
        .expect("scene item blend mode must be retrievable");
    assert_eq!(initial_blend_mode, ObswsSceneItemBlendMode::Normal);

    let set_result = registry
        .set_scene_item_blend_mode(
            OBSWS_DEFAULT_SCENE_NAME,
            scene_item_id,
            ObswsSceneItemBlendMode::Multiply,
        )
        .expect("set scene item blend mode must succeed");
    assert!(set_result.changed);

    let updated_blend_mode = registry
        .get_scene_item_blend_mode(OBSWS_DEFAULT_SCENE_NAME, scene_item_id)
        .expect("scene item blend mode must be retrievable");
    assert_eq!(updated_blend_mode, ObswsSceneItemBlendMode::Multiply);
}

#[test]
fn set_and_get_scene_item_transform_succeeds() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
            .expect("input settings must be valid");
    registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 0)
        .expect("scene item id must exist");

    let set_result = registry
        .set_scene_item_transform(
            OBSWS_DEFAULT_SCENE_NAME,
            scene_item_id,
            ObswsSceneItemTransformPatch {
                position_x: Some(123.0),
                position_y: Some(45.0),
                bounds_type: Some("OBS_BOUNDS_STRETCH".to_owned()),
                ..Default::default()
            },
        )
        .expect("set scene item transform must succeed");
    assert!(set_result.changed);

    let updated_transform = registry
        .get_scene_item_transform(OBSWS_DEFAULT_SCENE_NAME, scene_item_id)
        .expect("scene item transform must be retrievable");
    assert_eq!(updated_transform.position_x, 123.0);
    assert_eq!(updated_transform.position_y, 45.0);
    assert_eq!(updated_transform.bounds_type, "OBS_BOUNDS_STRETCH");
    assert_eq!(updated_transform.width, 0.0);
    assert_eq!(updated_transform.height, 0.0);
}

#[test]
fn create_scene_item_and_list_scene_items_succeed() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
    let input_1 =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
            .expect("input settings must be valid");
    let created_input_1 = registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input_1, false)
        .expect("input creation must succeed");
    let input_2 =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
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
    let input =
        ObswsInput::from_kind_and_settings("video_capture_device", parse_owned_json("{}").value())
            .expect("input settings must be valid");
    registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
        .expect("input creation must succeed");
    let scene_item_id = registry
        .get_scene_item_id(OBSWS_DEFAULT_SCENE_NAME, "camera-1", 0)
        .expect("scene item id must exist");
    registry
        .set_scene_item_locked(OBSWS_DEFAULT_SCENE_NAME, scene_item_id, true)
        .expect("set scene item locked must succeed");
    registry
        .set_scene_item_blend_mode(
            OBSWS_DEFAULT_SCENE_NAME,
            scene_item_id,
            ObswsSceneItemBlendMode::Screen,
        )
        .expect("set scene item blend mode must succeed");
    registry
        .set_scene_item_transform(
            OBSWS_DEFAULT_SCENE_NAME,
            scene_item_id,
            ObswsSceneItemTransformPatch {
                position_x: Some(77.0),
                ..Default::default()
            },
        )
        .expect("set scene item transform must succeed");

    let duplicated = registry
        .duplicate_scene_item(OBSWS_DEFAULT_SCENE_NAME, "Scene B", scene_item_id)
        .expect("scene item duplication must succeed");
    assert!(duplicated.scene_item.scene_item_id > scene_item_id);
    assert_eq!(duplicated.scene_name, "Scene B");
    assert_eq!(duplicated.scene_item.source_name, "camera-1");
    assert!(duplicated.scene_item.scene_item_locked);
    assert_eq!(
        duplicated.scene_item.scene_item_blend_mode,
        "OBS_BLEND_SCREEN"
    );

    let scene_b_items = registry
        .list_scene_items("Scene B")
        .expect("scene items must be listed");
    assert_eq!(scene_b_items.len(), 1);
    assert_eq!(scene_b_items[0].source_name, "camera-1");
    assert_eq!(scene_b_items[0].scene_item_blend_mode, "OBS_BLEND_SCREEN");

    let duplicated_transform = registry
        .get_scene_item_transform("Scene B", duplicated.scene_item.scene_item_id)
        .expect("duplicated scene item transform must be retrievable");
    assert_eq!(duplicated_transform.position_x, 77.0);
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
fn get_scene_uuid_returns_expected_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let created = registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    let scene_uuid = registry.get_scene_uuid("Scene B");
    assert_eq!(scene_uuid, Some(created.scene_uuid));
    assert!(registry.get_scene_uuid("not-found").is_none());
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
fn create_scene_and_set_current_preview_scene_succeeds() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let created = registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    assert_eq!(created.scene_name, "Scene B");

    registry
        .set_current_preview_scene("Scene B")
        .expect("setting current preview scene must succeed");
    assert_eq!(
        registry
            .current_preview_scene()
            .map(|scene| scene.scene_name),
        Some("Scene B".to_owned())
    );
}

#[test]
fn set_scene_name_updates_scene_and_current_scene_names() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    registry
        .set_current_program_scene("Scene B")
        .expect("setting current scene must succeed");
    registry
        .set_current_preview_scene("Scene B")
        .expect("setting current preview scene must succeed");

    let renamed = registry
        .set_scene_name("Scene B", "Scene Renamed")
        .expect("scene rename must succeed");
    assert_eq!(renamed.scene_name, "Scene Renamed");
    assert!(
        registry
            .list_scenes()
            .iter()
            .any(|scene| scene.scene_name == "Scene Renamed")
    );
    assert_eq!(
        registry
            .current_program_scene()
            .map(|scene| scene.scene_name),
        Some("Scene Renamed".to_owned())
    );
    assert_eq!(
        registry
            .current_preview_scene()
            .map(|scene| scene.scene_name),
        Some("Scene Renamed".to_owned())
    );
    let override_entry = registry
        .get_scene_transition_override("Scene Renamed")
        .expect("transition override lookup must succeed");
    assert_eq!(override_entry.transition_name, None);
    assert_eq!(override_entry.transition_duration, None);
}

#[test]
fn set_scene_name_rejects_duplicate_scene_name() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    let error = registry
        .set_scene_name(OBSWS_DEFAULT_SCENE_NAME, "Scene B")
        .expect_err("duplicate scene rename must fail");
    assert_eq!(error, SetSceneNameError::SceneNameAlreadyExists);
}

#[test]
fn is_source_active_returns_true_when_source_is_enabled_in_program_scene() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let input = ObswsInput::from_kind_and_settings(
        "video_capture_device",
        parse_owned_json(r#"{}"#).value(),
    )
    .expect("input settings must be valid");
    let created = registry
        .create_input(OBSWS_DEFAULT_SCENE_NAME, "camera-1", input, true)
        .expect("input creation must succeed");
    let active = registry
        .is_source_active(Some(&created.input_uuid), None)
        .expect("source lookup must succeed");
    assert!(active);
}

#[test]
fn scene_transition_override_round_trip_succeeds() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let override_entry = registry
        .set_scene_transition_override(OBSWS_DEFAULT_SCENE_NAME, Some("Fade"), Some(500))
        .expect("transition override update must succeed");
    assert_eq!(override_entry.transition_name.as_deref(), Some("Fade"));
    assert_eq!(override_entry.transition_duration, Some(500));

    let fetched = registry
        .get_scene_transition_override(OBSWS_DEFAULT_SCENE_NAME)
        .expect("transition override lookup must succeed");
    assert_eq!(fetched.transition_name.as_deref(), Some("Fade"));
    assert_eq!(fetched.transition_duration, Some(500));
}

#[test]
fn set_scene_name_moves_scene_transition_override_to_new_scene_name() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .set_scene_transition_override(OBSWS_DEFAULT_SCENE_NAME, Some("Fade"), Some(500))
        .expect("transition override update must succeed");

    registry
        .set_scene_name(OBSWS_DEFAULT_SCENE_NAME, "Scene Renamed")
        .expect("scene rename must succeed");

    let renamed_override = registry
        .get_scene_transition_override("Scene Renamed")
        .expect("renamed scene override lookup must succeed");
    assert_eq!(renamed_override.transition_name.as_deref(), Some("Fade"));
    assert_eq!(renamed_override.transition_duration, Some(500));
}

#[test]
fn transition_runtime_defaults_to_cut_and_300ms() {
    let registry = ObswsInputRegistry::new_for_test();
    assert_eq!(registry.current_scene_transition_name(), "Cut");
    assert_eq!(registry.current_scene_transition_duration_ms(), 300);
    assert_eq!(
        registry.current_scene_transition_settings().value().kind(),
        nojson::JsonValueKind::Object
    );
    assert_eq!(registry.current_tbar_position(), 0.0);
    assert_eq!(registry.supported_transition_kinds(), ["Cut", "Fade"]);
}

#[test]
fn set_current_scene_transition_updates_transition_name() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .set_current_scene_transition("Fade")
        .expect("setting transition to Fade must succeed");
    assert_eq!(registry.current_scene_transition_name(), "Fade");
}

#[test]
fn set_current_scene_transition_rejects_unknown_transition() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let error = registry
        .set_current_scene_transition("Swipe")
        .expect_err("unknown transition must be rejected");
    assert_eq!(error, SetCurrentSceneTransitionError::TransitionNotFound);
}

#[test]
fn set_current_scene_transition_duration_rejects_negative_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let error = registry
        .set_current_scene_transition_duration_ms(-1)
        .expect_err("negative transition duration must be rejected");
    assert_eq!(
        error,
        SetCurrentSceneTransitionDurationError::InvalidTransitionDuration
    );
}

#[test]
fn set_current_scene_transition_duration_rejects_zero_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let error = registry
        .set_current_scene_transition_duration_ms(0)
        .expect_err("zero transition duration must be rejected");
    assert_eq!(
        error,
        SetCurrentSceneTransitionDurationError::InvalidTransitionDuration
    );
}

#[test]
fn set_current_scene_transition_duration_rejects_too_large_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let error = registry
        .set_current_scene_transition_duration_ms(20_001)
        .expect_err("too large transition duration must be rejected");
    assert_eq!(
        error,
        SetCurrentSceneTransitionDurationError::InvalidTransitionDuration
    );
}

#[test]
fn set_current_scene_transition_duration_updates_runtime_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .set_current_scene_transition_duration_ms(500)
        .expect("transition duration update must succeed");
    assert_eq!(registry.current_scene_transition_duration_ms(), 500);
}

#[test]
fn set_current_scene_transition_settings_updates_runtime_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let transition_settings = nojson::RawJsonOwned::parse(r#"{"speed":2,"style":"smooth"}"#)
        .expect("transition settings must be valid json");
    registry
        .set_current_scene_transition_settings(transition_settings)
        .expect("transition settings update must succeed");
    let speed: i64 = registry
        .current_scene_transition_settings()
        .value()
        .to_member("speed")
        .and_then(|v| v.required()?.try_into())
        .expect("transition settings speed must exist");
    assert_eq!(speed, 2);
}

#[test]
fn set_current_scene_transition_settings_rejects_non_object_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let transition_settings =
        nojson::RawJsonOwned::parse(r#""invalid""#).expect("json must be valid");
    let error = registry
        .set_current_scene_transition_settings(transition_settings)
        .expect_err("non-object transition settings must be rejected");
    assert_eq!(
        error,
        SetCurrentSceneTransitionSettingsError::InvalidTransitionSettings
    );
}

#[test]
fn set_tbar_position_updates_runtime_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .set_tbar_position(0.25)
        .expect("tbar position update must succeed");
    assert_eq!(registry.current_tbar_position(), 0.25);
}

#[test]
fn set_tbar_position_rejects_out_of_range_value() {
    let mut registry = ObswsInputRegistry::new_for_test();
    let error = registry
        .set_tbar_position(1.5)
        .expect_err("out-of-range tbar position must be rejected");
    assert_eq!(error, SetTBarPositionError::InvalidTBarPosition);
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
fn remove_scene_switches_current_preview_scene_when_current_is_removed() {
    let mut registry = ObswsInputRegistry::new_for_test();
    registry
        .create_scene("Scene B")
        .expect("scene creation must succeed");
    registry
        .set_current_preview_scene("Scene B")
        .expect("setting preview scene must succeed");

    registry
        .remove_scene("Scene B")
        .expect("scene removal must succeed");
    assert_eq!(
        registry
            .current_preview_scene()
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
            video: Some(ObswsRecordTrackRun {
                encoder_processor_id: "encoder".to_owned(),
                source_track_id: "source-track".to_owned(),
                encoded_track_id: "encoded-track".to_owned(),
            }),
            audio: None,
            publisher_processor_id: "publisher".to_owned(),
        })
        .expect("stream activation must succeed");
    assert!(registry.is_stream_active());

    registry.deactivate_stream();
    assert!(!registry.is_stream_active());
    assert_eq!(registry.stream_uptime(), Duration::ZERO);
}

#[test]
fn record_runtime_state_changes_on_activate_pause_resume_and_deactivate() {
    let mut registry = ObswsInputRegistry::new_for_test();
    assert!(!registry.is_record_active());
    assert!(!registry.is_record_paused());
    assert_eq!(registry.record_uptime(), Duration::ZERO);

    registry
        .activate_record(ObswsRecordRun {
            source_processor_id: "source".to_owned(),
            video: Some(ObswsRecordTrackRun {
                encoder_processor_id: "encoder".to_owned(),
                source_track_id: "source-track".to_owned(),
                encoded_track_id: "encoded-track".to_owned(),
            }),
            audio: None,
            writer_processor_id: "writer".to_owned(),
            output_path: PathBuf::from("recordings-for-test/output.mp4"),
        })
        .expect("record activation must succeed");
    assert!(registry.is_record_active());
    assert!(!registry.is_record_paused());

    registry.pause_record().expect("record pause must succeed");
    assert!(registry.is_record_paused());

    registry
        .resume_record()
        .expect("record resume must succeed");
    assert!(!registry.is_record_paused());

    registry.deactivate_record();
    assert!(!registry.is_record_active());
    assert!(!registry.is_record_paused());
    assert_eq!(registry.record_uptime(), Duration::ZERO);
}

#[test]
fn record_pause_resume_returns_expected_errors() {
    let mut registry = ObswsInputRegistry::new_for_test();
    assert_eq!(
        registry.pause_record(),
        Err(PauseRecordError::RecordNotActive)
    );
    assert_eq!(
        registry.resume_record(),
        Err(ResumeRecordError::RecordNotActive)
    );

    registry
        .activate_record(ObswsRecordRun {
            source_processor_id: "source".to_owned(),
            video: Some(ObswsRecordTrackRun {
                encoder_processor_id: "encoder".to_owned(),
                source_track_id: "source-track".to_owned(),
                encoded_track_id: "encoded-track".to_owned(),
            }),
            audio: None,
            writer_processor_id: "writer".to_owned(),
            output_path: PathBuf::from("recordings-for-test/output.mp4"),
        })
        .expect("record activation must succeed");
    assert_eq!(registry.resume_record(), Err(ResumeRecordError::NotPaused));
    registry.pause_record().expect("record pause must succeed");
    assert_eq!(
        registry.pause_record(),
        Err(PauseRecordError::AlreadyPaused)
    );
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
