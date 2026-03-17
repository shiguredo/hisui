use super::*;

impl ObswsSession {
    pub(super) async fn handle_set_current_program_scene_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let previous_scene_name = input_registry
            .current_program_scene()
            .map(|scene| scene.scene_name);
        let response_text = crate::obsws_response_builder::build_set_current_program_scene_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENES) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(current_scene) = input_registry.current_program_scene() else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        if previous_scene_name.as_deref() == Some(current_scene.scene_name.as_str()) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }

        let event_text = crate::obsws_response_builder::build_current_program_scene_changed_event(
            &current_scene.scene_name,
            &current_scene.scene_uuid,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    pub(super) async fn handle_set_current_preview_scene_request(
        &self,
        request_id: &str,
        _request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let response_text =
            crate::obsws_response_builder::build_set_current_preview_scene_response(request_id);
        SessionAction::SendText {
            text: response_text,
            message_name: "request response message",
        }
    }

    pub(super) async fn handle_create_scene_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let requested_scene_name =
            Self::parse_required_non_empty_string_request_field(request_data, "sceneName");
        let existed_before = requested_scene_name.as_deref().is_some_and(|scene_name| {
            input_registry
                .list_scenes()
                .into_iter()
                .any(|scene| scene.scene_name == scene_name)
        });
        let response_text = crate::obsws_response_builder::build_create_scene_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENES) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        if existed_before {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(requested_scene_name) = requested_scene_name else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let Some(created_scene) = input_registry
            .list_scenes()
            .into_iter()
            .find(|scene| scene.scene_name == requested_scene_name)
        else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };

        let event_text = crate::obsws_response_builder::build_scene_created_event(
            &created_scene.scene_name,
            &created_scene.scene_uuid,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    pub(super) async fn handle_remove_scene_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        // sceneName または sceneUuid からシーンを解決する
        let removed_scene = request_data.and_then(|rd| {
            let (scene_name, scene_uuid) =
                crate::obsws_response_builder::parse_scene_lookup_fields_for_session(
                    rd.value(),
                    "sceneName",
                    "sceneUuid",
                )
                .ok()?;
            let resolved_name =
                input_registry.resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())?;
            input_registry
                .list_scenes()
                .into_iter()
                .find(|scene| scene.scene_name == resolved_name)
        });
        let previous_current_scene_name = input_registry
            .current_program_scene()
            .map(|scene| scene.scene_name);
        let response_text = crate::obsws_response_builder::build_remove_scene_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENES) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(removed_scene) = removed_scene else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let removed_succeeded = input_registry
            .list_scenes()
            .into_iter()
            .all(|scene| scene.scene_uuid != removed_scene.scene_uuid);
        if !removed_succeeded {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }

        let mut messages = vec![
            (response_text, "request response message"),
            (
                crate::obsws_response_builder::build_scene_removed_event(
                    &removed_scene.scene_name,
                    &removed_scene.scene_uuid,
                ),
                "event message",
            ),
        ];
        if previous_current_scene_name.as_deref() == Some(removed_scene.scene_name.as_str())
            && let Some(current_scene) = input_registry.current_program_scene()
        {
            messages.push((
                crate::obsws_response_builder::build_current_program_scene_changed_event(
                    &current_scene.scene_name,
                    &current_scene.scene_uuid,
                ),
                "event message",
            ));
        }
        SessionAction::SendTexts { messages }
    }
}
