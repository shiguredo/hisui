use super::*;

impl ObswsSession {
    pub(super) async fn handle_create_scene_item_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let execution = crate::obsws_response_builder::execute_create_scene_item(
            request_id,
            request_data,
            &mut input_registry,
        );
        let response_text = execution.response_text;
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEMS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(created_scene_item) = execution.created else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };

        let event_text = crate::obsws_response_builder::build_scene_item_created_event(
            &created_scene_item.scene_name,
            &created_scene_item.scene_uuid,
            created_scene_item.scene_item.scene_item_id,
            &created_scene_item.scene_item.source_name,
            &created_scene_item.scene_item.source_uuid,
            created_scene_item.scene_item.scene_item_index,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    pub(super) async fn handle_remove_scene_item_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let Some(request_data) = request_data else {
            return Self::build_missing_request_data_error_action("RemoveSceneItem", request_id);
        };
        let (scene_name, scene_uuid) =
            match crate::obsws_response_builder::parse_scene_lookup_fields_for_session(
                request_data.value(),
                "sceneName",
                "sceneUuid",
            ) {
                Ok(fields) => fields,
                Err(error) => {
                    return Self::build_parse_error_action("RemoveSceneItem", request_id, &error);
                }
            };
        let scene_item_id =
            match crate::obsws_response_builder::parse_required_i64_field_for_session(
                request_data.value(),
                "sceneItemId",
            ) {
                Ok(value) => value,
                Err(error) => {
                    return Self::build_parse_error_action("RemoveSceneItem", request_id, &error);
                }
            };
        let target_fields = input_registry
            .resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())
            .map(|scene_name| {
                let scene_uuid = input_registry
                    .get_scene_uuid(&scene_name)
                    .unwrap_or_default();
                (scene_name, scene_uuid, scene_item_id)
            });
        let removed_scene_item =
            target_fields
                .as_ref()
                .and_then(|(scene_name, _, scene_item_id)| {
                    let (source_name, source_uuid) = input_registry
                        .get_scene_item_source(scene_name, *scene_item_id)
                        .ok()?;
                    Some((source_name, source_uuid))
                });
        let scene_items_before = target_fields.as_ref().and_then(|(scene_name, _, _)| {
            input_registry
                .list_scene_items(scene_name)
                .ok()
                .map(|scene_items| {
                    scene_items
                        .iter()
                        .map(|scene_item| (scene_item.scene_item_id, scene_item.scene_item_index))
                        .collect::<Vec<_>>()
                })
        });
        let response_text = crate::obsws_response_builder::build_remove_scene_item_response(
            request_id,
            Some(request_data),
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEMS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some((scene_name, scene_uuid, scene_item_id)) = target_fields else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let Some((source_name, source_uuid)) = removed_scene_item else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };

        let mut messages = vec![
            (response_text, "request response message"),
            (
                crate::obsws_response_builder::build_scene_item_removed_event(
                    &scene_name,
                    &scene_uuid,
                    scene_item_id,
                    &source_name,
                    &source_uuid,
                ),
                "event message",
            ),
        ];

        let scene_items_after = input_registry
            .list_scene_items(&scene_name)
            .unwrap_or_default()
            .iter()
            .map(
                |scene_item| crate::obsws_input_registry::ObswsSceneItemIndexEntry {
                    scene_item_id: scene_item.scene_item_id,
                    scene_item_index: scene_item.scene_item_index,
                },
            )
            .collect::<Vec<_>>();
        let scene_items_after_simple = scene_items_after
            .iter()
            .map(|scene_item| (scene_item.scene_item_id, scene_item.scene_item_index))
            .collect::<Vec<_>>();
        if let Some(scene_items_before) = scene_items_before {
            let still_present_before = scene_items_before
                .into_iter()
                .filter(|(scene_item_id, _)| {
                    scene_items_after_simple
                        .iter()
                        .any(|(after_scene_item_id, _)| after_scene_item_id == scene_item_id)
                })
                .collect::<Vec<_>>();
            if still_present_before != scene_items_after_simple {
                messages.push((
                    crate::obsws_response_builder::build_scene_item_list_reindexed_event(
                        &scene_name,
                        &scene_uuid,
                        &scene_items_after,
                    ),
                    "event message",
                ));
            }
        }

        SessionAction::SendTexts { messages }
    }

    pub(super) async fn handle_duplicate_scene_item_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let execution = crate::obsws_response_builder::execute_duplicate_scene_item(
            request_id,
            request_data,
            &mut input_registry,
        );
        let response_text = execution.response_text;
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEMS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(duplicated_scene_item) = execution.duplicated else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };

        let event_text = crate::obsws_response_builder::build_scene_item_created_event(
            &duplicated_scene_item.scene_name,
            &duplicated_scene_item.scene_uuid,
            duplicated_scene_item.scene_item.scene_item_id,
            &duplicated_scene_item.scene_item.source_name,
            &duplicated_scene_item.scene_item.source_uuid,
            duplicated_scene_item.scene_item.scene_item_index,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    pub(super) async fn handle_set_scene_item_index_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let execution = crate::obsws_response_builder::execute_set_scene_item_index(
            request_id,
            request_data,
            &mut input_registry,
        );
        let response_text = execution.response_text;
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEMS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(event_context) = execution.event_context else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        if !event_context.set_result.changed {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (
                    crate::obsws_response_builder::build_scene_item_list_reindexed_event(
                        &event_context.scene_name,
                        &event_context.scene_uuid,
                        &event_context.set_result.scene_items,
                    ),
                    "event message",
                ),
            ],
        }
    }

    pub(super) async fn handle_set_scene_item_enabled_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let Some(request_data) = request_data else {
            return Self::build_missing_request_data_error_action(
                "SetSceneItemEnabled",
                request_id,
            );
        };
        let requested_fields =
            match crate::obsws_response_builder::parse_set_scene_item_enabled_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => Some(fields),
                Err(error) => {
                    return Self::build_parse_error_action(
                        "SetSceneItemEnabled",
                        request_id,
                        &error,
                    );
                }
            };
        let previous_scene_item_enabled =
            requested_fields
                .as_ref()
                .and_then(|(scene_name, scene_item_id, _)| {
                    input_registry
                        .get_scene_item_enabled(scene_name, *scene_item_id)
                        .ok()
                });
        let response_text = crate::obsws_response_builder::build_set_scene_item_enabled_response(
            request_id,
            Some(request_data),
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEMS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some((scene_name, scene_item_id, scene_item_enabled)) = requested_fields else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let Some(previous_scene_item_enabled) = previous_scene_item_enabled else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        if previous_scene_item_enabled == scene_item_enabled {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let scene_uuid = input_registry
            .get_scene_uuid(&scene_name)
            .unwrap_or_default();

        let event_text = crate::obsws_response_builder::build_scene_item_enable_state_changed_event(
            &scene_name,
            &scene_uuid,
            scene_item_id,
            scene_item_enabled,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    pub(super) async fn handle_set_scene_item_locked_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let execution = crate::obsws_response_builder::execute_set_scene_item_locked(
            request_id,
            request_data,
            &mut input_registry,
        );
        let response_text = execution.response_text;
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEMS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(event_context) = execution.event_context else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        if !event_context.set_result.changed {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let event_text = crate::obsws_response_builder::build_scene_item_lock_state_changed_event(
            &event_context.scene_name,
            &event_context.scene_uuid,
            event_context.scene_item_id,
            event_context.scene_item_locked,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    pub(super) async fn handle_set_scene_item_blend_mode_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let response_text = crate::obsws_response_builder::build_set_scene_item_blend_mode_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        SessionAction::SendText {
            text: response_text,
            message_name: "request response message",
        }
    }

    pub(super) async fn handle_set_scene_item_transform_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let execution = crate::obsws_response_builder::execute_set_scene_item_transform(
            request_id,
            request_data,
            &mut input_registry,
        );
        let response_text = execution.response_text;
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEMS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(event_context) = execution.event_context else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        if !event_context.set_result.changed {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let event_text = crate::obsws_response_builder::build_scene_item_transform_changed_event(
            &event_context.scene_name,
            &event_context.scene_uuid,
            event_context.scene_item_id,
            &event_context.set_result.scene_item_transform,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }
}
