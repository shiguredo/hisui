use super::*;

impl ObswsSession {
    pub(super) async fn handle_create_input_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let execution = crate::obsws_response_builder::execute_create_input(
            request_id,
            request_data,
            &mut input_registry,
        );
        let response_text = execution.response_text;
        let Some(created) = execution.created else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };

        let input_event_enabled = self.is_event_subscription_enabled(OBSWS_EVENT_SUB_INPUTS);
        let scene_item_event_enabled =
            self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEMS);
        let transform_event_enabled =
            self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED);

        if !input_event_enabled && !scene_item_event_enabled && !transform_event_enabled {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }

        let mut messages = vec![(response_text, "request response message")];

        // OBS の送信順序に合わせ、InputCreated → SceneItemCreated の順で送信
        if input_event_enabled {
            let event_text = crate::obsws_response_builder::build_input_created_event(
                &created.input_entry.input_name,
                &created.input_entry.input_uuid,
                created.input_entry.input.kind_name(),
                &created.input_entry.input.settings,
                &created.default_settings,
            );
            messages.push((event_text, "event message"));
        }

        if scene_item_event_enabled {
            let scene_item = &created.scene_item_ref;
            let event_text = crate::obsws_response_builder::build_scene_item_created_event(
                &scene_item.scene_name,
                &scene_item.scene_uuid,
                scene_item.scene_item.scene_item_id,
                &scene_item.scene_item.source_name,
                &scene_item.scene_item.source_uuid,
                scene_item.scene_item.scene_item_index,
            );
            messages.push((event_text, "event message"));
        }

        // OBS は CreateInput でシーンアイテムが作成された際にデフォルト transform の
        // SceneItemTransformChanged イベントを発火する
        if transform_event_enabled {
            let scene_item = &created.scene_item_ref;
            let event_text =
                crate::obsws_response_builder::build_scene_item_transform_changed_event(
                    &scene_item.scene_name,
                    &scene_item.scene_uuid,
                    scene_item.scene_item.scene_item_id,
                    &scene_item.scene_item.scene_item_transform,
                );
            messages.push((event_text, "event message"));
        }

        SessionAction::SendTexts { messages }
    }

    pub(super) async fn handle_remove_input_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let Some(request_data) = request_data else {
            return Self::build_missing_request_data_error_action("RemoveInput", request_id);
        };
        let (input_uuid, input_name) =
            match crate::obsws_response_builder::parse_input_lookup_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => fields,
                Err(error) => {
                    return Self::build_parse_error_action("RemoveInput", request_id, &error);
                }
            };
        let removed_input = input_registry
            .find_input(input_uuid.as_deref(), input_name.as_deref())
            .cloned();

        // 削除前に、この input を参照するシーンアイテムを収集する（SceneItemRemoved イベント用）
        let scene_items_to_remove = removed_input
            .as_ref()
            .map(|input| input_registry.find_scene_items_by_input_uuid(&input.input_uuid));

        let response_text = crate::obsws_response_builder::build_remove_input_response(
            request_id,
            Some(request_data),
            &mut input_registry,
        );

        let input_event_enabled = self.is_event_subscription_enabled(OBSWS_EVENT_SUB_INPUTS);
        let scene_item_event_enabled =
            self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENE_ITEMS);

        if !input_event_enabled && !scene_item_event_enabled {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(removed_input) = removed_input else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let removed_succeeded = input_registry
            .find_input(Some(&removed_input.input_uuid), None)
            .is_none();
        if !removed_succeeded {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }

        let mut messages = vec![(response_text, "request response message")];

        // OBS の送信順序に合わせ、InputRemoved → SceneItemRemoved の順で送信
        if input_event_enabled {
            let event_text = crate::obsws_response_builder::build_input_removed_event(
                &removed_input.input_name,
                &removed_input.input_uuid,
            );
            messages.push((event_text, "event message"));
        }

        if scene_item_event_enabled && let Some(scene_items) = scene_items_to_remove {
            for (scene_name, scene_uuid, scene_item_id) in scene_items {
                let event_text = crate::obsws_response_builder::build_scene_item_removed_event(
                    &scene_name,
                    &scene_uuid,
                    scene_item_id,
                    &removed_input.input_name,
                    &removed_input.input_uuid,
                );
                messages.push((event_text, "event message"));
            }
        }

        SessionAction::SendTexts { messages }
    }

    pub(super) async fn handle_set_input_settings_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let Some(request_data) = request_data else {
            return Self::build_missing_request_data_error_action("SetInputSettings", request_id);
        };
        let requested_input_lookup =
            match crate::obsws_response_builder::parse_input_lookup_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => Some(fields),
                Err(error) => {
                    return Self::build_parse_error_action("SetInputSettings", request_id, &error);
                }
            };
        let execution = crate::obsws_response_builder::execute_set_input_settings(
            request_id,
            Some(request_data),
            &mut input_registry,
        );
        let response_text = execution.response_text;
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_INPUTS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        if !execution.request_succeeded {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some((input_uuid, input_name)) = requested_input_lookup else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let Some(updated_input) = input_registry
            .find_input(input_uuid.as_deref(), input_name.as_deref())
            .cloned()
        else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let event_text = crate::obsws_response_builder::build_input_settings_changed_event(
            &updated_input.input_name,
            &updated_input.input_uuid,
            &updated_input.input.settings,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    pub(super) async fn handle_set_input_name_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let Some(request_data) = request_data else {
            return Self::build_missing_request_data_error_action("SetInputName", request_id);
        };
        let requested_input_lookup =
            match crate::obsws_response_builder::parse_input_lookup_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => Some(fields),
                Err(error) => {
                    return Self::build_parse_error_action("SetInputName", request_id, &error);
                }
            };
        let old_input = requested_input_lookup
            .as_ref()
            .and_then(|(input_uuid, input_name)| {
                input_registry
                    .find_input(input_uuid.as_deref(), input_name.as_deref())
                    .cloned()
            });
        let response_text = crate::obsws_response_builder::build_set_input_name_response(
            request_id,
            Some(request_data),
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_INPUTS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(old_input) = old_input else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let Some(updated_input) = input_registry
            .find_input(Some(&old_input.input_uuid), None)
            .cloned()
        else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        if old_input.input_name == updated_input.input_name {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let event_text = crate::obsws_response_builder::build_input_name_changed_event(
            &updated_input.input_name,
            &old_input.input_name,
            &updated_input.input_uuid,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }
}
