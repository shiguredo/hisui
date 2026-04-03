//! SceneItem CRUD ハンドラを定義するモジュール。
//! input_registry 内のシーンに属するシーンアイテムを変更し、
//! シーンアイテム関連の obsws イベントを発行する。

use super::{CommandResult, ObswsCoordinator};
use crate::obsws::event::TaggedEvent;
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED, OBSWS_EVENT_SUB_SCENE_ITEMS,
    REQUEST_STATUS_MISSING_REQUEST_DATA,
};

impl ObswsCoordinator {
    pub(crate) fn handle_create_scene_item(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_create_scene_item(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(created_scene_item) = execution.created {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_created_event(
                    &created_scene_item.scene_name,
                    &created_scene_item.scene_uuid,
                    created_scene_item.scene_item.scene_item_id,
                    &created_scene_item.scene_item.source_name,
                    &created_scene_item.scene_item.source_uuid,
                    created_scene_item.scene_item.scene_item_index,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) fn handle_remove_scene_item(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "RemoveSceneItem",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let (scene_name, scene_uuid) =
            match crate::obsws::response::parse_scene_lookup_fields_for_session(
                request_data.value(),
                "sceneName",
                "sceneUuid",
            ) {
                Ok(fields) => fields,
                Err(error) => {
                    return self.build_parse_error_result("RemoveSceneItem", request_id, &error);
                }
            };
        let scene_item_id = match crate::obsws::response::parse_required_i64_field_for_session(
            request_data.value(),
            "sceneItemId",
        ) {
            Ok(value) => value,
            Err(error) => {
                return self.build_parse_error_result("RemoveSceneItem", request_id, &error);
            }
        };
        let target_fields = self
            .input_registry
            .resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())
            .map(|scene_name| {
                let scene_uuid = self
                    .input_registry
                    .get_scene_uuid(&scene_name)
                    .unwrap_or_default();
                (scene_name, scene_uuid, scene_item_id)
            });
        let removed_scene_item =
            target_fields
                .as_ref()
                .and_then(|(scene_name, _, scene_item_id)| {
                    let (source_name, source_uuid) = self
                        .input_registry
                        .get_scene_item_source(scene_name, *scene_item_id)
                        .ok()?;
                    Some((source_name, source_uuid))
                });
        let scene_items_before = target_fields.as_ref().and_then(|(scene_name, _, _)| {
            self.input_registry
                .list_scene_items(scene_name)
                .ok()
                .map(|scene_items| {
                    scene_items
                        .iter()
                        .map(|si| (si.scene_item_id, si.scene_item_index))
                        .collect::<Vec<_>>()
                })
        });
        let response_text = crate::obsws::response::build_remove_scene_item_response(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some((scene_name, scene_uuid, scene_item_id)) = target_fields
            && let Some((source_name, source_uuid)) = removed_scene_item
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_removed_event(
                    &scene_name,
                    &scene_uuid,
                    scene_item_id,
                    &source_name,
                    &source_uuid,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
            let scene_items_after = self
                .input_registry
                .list_scene_items(&scene_name)
                .unwrap_or_default()
                .iter()
                .map(
                    |si| crate::obsws::input_registry::ObswsSceneItemIndexEntry {
                        scene_item_id: si.scene_item_id,
                        scene_item_index: si.scene_item_index,
                    },
                )
                .collect::<Vec<_>>();
            let scene_items_after_simple = scene_items_after
                .iter()
                .map(|si| (si.scene_item_id, si.scene_item_index))
                .collect::<Vec<_>>();
            if let Some(scene_items_before) = scene_items_before {
                let still_present_before = scene_items_before
                    .into_iter()
                    .filter(|(id, _)| {
                        scene_items_after_simple
                            .iter()
                            .any(|(after_id, _)| after_id == id)
                    })
                    .collect::<Vec<_>>();
                if still_present_before != scene_items_after_simple {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_scene_item_list_reindexed_event(
                            &scene_name,
                            &scene_uuid,
                            &scene_items_after,
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
                    });
                }
            }
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) fn handle_duplicate_scene_item(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_duplicate_scene_item(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(duplicated) = execution.duplicated {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_created_event(
                    &duplicated.scene_name,
                    &duplicated.scene_uuid,
                    duplicated.scene_item.scene_item_id,
                    &duplicated.scene_item.source_name,
                    &duplicated.scene_item.source_uuid,
                    duplicated.scene_item.scene_item_index,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) fn handle_set_scene_item_enabled(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "SetSceneItemEnabled",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let requested_fields =
            match crate::obsws::response::parse_set_scene_item_enabled_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => Some(fields),
                Err(error) => {
                    return self.build_parse_error_result(
                        "SetSceneItemEnabled",
                        request_id,
                        &error,
                    );
                }
            };
        let previous_enabled =
            requested_fields
                .as_ref()
                .and_then(|(scene_name, scene_uuid, scene_item_id, _)| {
                    let resolved_name = self
                        .input_registry
                        .resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())?;
                    self.input_registry
                        .get_scene_item_enabled(&resolved_name, *scene_item_id)
                        .ok()
                });
        let response_text = crate::obsws::response::build_set_scene_item_enabled_response(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some((scene_name, scene_uuid, scene_item_id, scene_item_enabled)) = requested_fields
            && let Some(prev) = previous_enabled
            && prev != scene_item_enabled
        {
            let resolved_scene_name = self
                .input_registry
                .resolve_scene_name(scene_name.as_deref(), scene_uuid.as_deref())
                .unwrap_or_default();
            let resolved_scene_uuid = self
                .input_registry
                .get_scene_uuid(&resolved_scene_name)
                .unwrap_or_default();
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_enable_state_changed_event(
                    &resolved_scene_name,
                    &resolved_scene_uuid,
                    scene_item_id,
                    scene_item_enabled,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) fn handle_set_scene_item_locked(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_set_scene_item_locked(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(ctx) = execution.event_context
            && ctx.set_result.changed
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_lock_state_changed_event(
                    &ctx.scene_name,
                    &ctx.scene_uuid,
                    ctx.scene_item_id,
                    ctx.scene_item_locked,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) fn handle_set_scene_item_index(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_set_scene_item_index(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(ctx) = execution.event_context {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_list_reindexed_event(
                    &ctx.scene_name,
                    &ctx.scene_uuid,
                    &ctx.set_result.scene_items,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) fn handle_set_scene_item_blend_mode(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let response_text = crate::obsws::response::build_set_scene_item_blend_mode_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        self.build_result_from_response(response_text, Vec::new())
    }

    pub(crate) fn handle_set_scene_item_transform(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_set_scene_item_transform(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(ctx) = execution.event_context
            && ctx.set_result.changed
        {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_transform_changed_event(
                    &ctx.scene_name,
                    &ctx.scene_uuid,
                    ctx.scene_item_id,
                    &ctx.set_result.scene_item_transform,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED,
            });
        }
        self.build_result_from_response(response_text, events)
    }
}
