//! Scene CRUD ハンドラを定義するモジュール。
//! input_registry のシーン状態を変更し、シーン関連の obsws イベントを発行する。

use super::{CommandResult, ObswsCoordinator, parse_required_non_empty_string_field};
use crate::obsws::event::TaggedEvent;
use crate::obsws::protocol::OBSWS_EVENT_SUB_SCENES;

impl ObswsCoordinator {
    pub(crate) fn handle_set_current_program_scene(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let previous_scene_name = self
            .input_registry
            .current_program_scene()
            .map(|scene| scene.scene_name);
        let response_text = crate::obsws::response::build_set_current_program_scene_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some(current_scene) = self.input_registry.current_program_scene()
            && previous_scene_name.as_deref() != Some(current_scene.scene_name.as_str())
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_current_program_scene_changed_event(
                    &current_scene.scene_name,
                    &current_scene.scene_uuid,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENES,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) fn handle_create_scene(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let requested_scene_name = parse_required_non_empty_string_field(request_data, "sceneName");
        let existed_before = requested_scene_name.as_deref().is_some_and(|scene_name| {
            self.input_registry
                .list_scenes()
                .into_iter()
                .any(|scene| scene.scene_name == scene_name)
        });
        let response_text = crate::obsws::response::build_create_scene_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if !existed_before
            && let Some(requested_scene_name) = requested_scene_name
            && let Some(created_scene) = self
                .input_registry
                .list_scenes()
                .into_iter()
                .find(|scene| scene.scene_name == requested_scene_name)
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_created_event(
                    &created_scene.scene_name,
                    &created_scene.scene_uuid,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENES,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) fn handle_remove_scene(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let removed_scene = request_data.and_then(|rd| {
            let (scene_name, scene_uuid) =
                crate::obsws::response::parse_scene_lookup_fields_for_session(
                    rd.value(),
                    "sceneName",
                    "sceneUuid",
                )
                .ok()?;
            let resolved_name = self
                .input_registry
                .resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())?;
            self.input_registry
                .list_scenes()
                .into_iter()
                .find(|scene| scene.scene_name == resolved_name)
        });
        let previous_current_scene_name = self
            .input_registry
            .current_program_scene()
            .map(|scene| scene.scene_name);
        let response_text = crate::obsws::response::build_remove_scene_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some(removed_scene) = removed_scene {
            let removed_succeeded = self
                .input_registry
                .list_scenes()
                .into_iter()
                .all(|scene| scene.scene_uuid != removed_scene.scene_uuid);
            if removed_succeeded {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_scene_removed_event(
                        &removed_scene.scene_name,
                        &removed_scene.scene_uuid,
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_SCENES,
                });
                if previous_current_scene_name.as_deref() == Some(removed_scene.scene_name.as_str())
                    && let Some(current_scene) = self.input_registry.current_program_scene()
                {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_current_program_scene_changed_event(
                            &current_scene.scene_name,
                            &current_scene.scene_uuid,
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_SCENES,
                    });
                }
            }
        }
        self.build_result_from_response(response_text, events)
    }
}
