use super::*;

impl ObswsSession {
    pub(super) async fn handle_create_input_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let requested_input_name =
            Self::parse_required_non_empty_string_request_field(request_data, "inputName");
        let existed_before = requested_input_name
            .as_deref()
            .is_some_and(|input_name| input_registry.find_input(None, Some(input_name)).is_some());
        let response_text = crate::obsws_response_builder::build_create_input_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_INPUTS) {
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
        let Some(requested_input_name) = requested_input_name else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let Some(created_input) = input_registry
            .find_input(None, Some(requested_input_name.as_str()))
            .cloned()
        else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let Ok(default_settings) =
            input_registry.get_input_default_settings(created_input.input.kind_name())
        else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let event_text = crate::obsws_response_builder::build_input_created_event(
            &created_input.input_name,
            &created_input.input_uuid,
            created_input.input.kind_name(),
            &created_input.input.settings,
            &default_settings,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
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
        let response_text = crate::obsws_response_builder::build_remove_input_response(
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

        let event_text = crate::obsws_response_builder::build_input_removed_event(
            &removed_input.input_name,
            &removed_input.input_uuid,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
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
