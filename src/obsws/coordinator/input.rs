//! Input および Media Input リクエストハンドラを扱うモジュール。
//! 各入力に対する source processor のライフサイクル管理（起動・停止）と、
//! audio mixer のミュート・音量状態の同期も担当する。

use super::{BootstrapInputEvent, BootstrapInputSnapshot, CommandResult, InputSourceState};
use crate::obsws::event::TaggedEvent;
use crate::obsws::protocol::*;

impl super::ObswsCoordinator {
    // -----------------------------------------------------------------------
    // Input 系ハンドラ
    // -----------------------------------------------------------------------

    pub(crate) async fn handle_create_input(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::execute_create_input(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if let Some(created) = execution.created {
            // 起動条件を満たしている場合のみ source processor を起動する
            if crate::obsws::source::is_source_startable(&created.input_entry.input.settings)
                && let Err(e) = self
                    .start_input_source_processor(&created.input_entry)
                    .await
            {
                tracing::warn!(
                    "failed to start source processor for input {}: {}",
                    created.input_entry.input_uuid,
                    e.display()
                );
            }

            // bootstrap 用の差分イベントを送信する
            if let Some(source_state) = self
                .input_source_processors
                .get(&created.input_entry.input_uuid)
            {
                let _ = self
                    .bootstrap_event_tx
                    .send(BootstrapInputEvent::InputCreated(BootstrapInputSnapshot {
                        input_uuid: created.input_entry.input_uuid.clone(),
                        input_name: created.input_entry.input_name.clone(),
                        input_kind: created.input_entry.input.kind_name().to_owned(),
                        video_track_id: source_state.video_track_id.clone(),
                        audio_track_id: source_state.audio_track_id.clone(),
                    }));
            }

            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_created_event(
                    &created.input_entry.input_name,
                    &created.input_entry.input_uuid,
                    created.input_entry.input.kind_name(),
                    &created.input_entry.input.settings,
                    &created.default_settings,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
            let scene_item = &created.scene_item_ref;
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_created_event(
                    &scene_item.scene_name,
                    &scene_item.scene_uuid,
                    scene_item.scene_item.scene_item_id,
                    &scene_item.scene_item.source_name,
                    &scene_item.scene_item.source_uuid,
                    scene_item.scene_item.scene_item_index,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_scene_item_transform_changed_event(
                    &scene_item.scene_name,
                    &scene_item.scene_uuid,
                    scene_item.scene_item.scene_item_id,
                    &scene_item.scene_item.scene_item_transform,
                ),
                subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED,
            });
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) async fn handle_remove_input(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "RemoveInput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let (input_uuid, input_name) =
            match crate::obsws::response::parse_input_lookup_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => fields,
                Err(error) => {
                    return self.build_parse_error_result("RemoveInput", request_id, &error);
                }
            };
        let removed_input = self
            .input_registry
            .find_input(input_uuid.as_deref(), input_name.as_deref())
            .cloned();
        // 削除前にシーンアイテムを収集する（イベント用）
        let scene_items_to_remove = removed_input.as_ref().map(|input| {
            self.input_registry
                .find_scene_items_by_input_uuid(&input.input_uuid)
        });
        let response_text = crate::obsws::response::build_remove_input_response(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some(removed_input) = removed_input {
            let removed_succeeded = self
                .input_registry
                .find_input(Some(&removed_input.input_uuid), None)
                .is_none();
            if removed_succeeded {
                // 入力ライフサイクルの source processor を停止する
                if let Err(e) = self
                    .stop_input_source_processor(&removed_input.input_uuid)
                    .await
                {
                    tracing::warn!(
                        "failed to stop source processor for input {}: {}",
                        removed_input.input_uuid,
                        e.display()
                    );
                }

                // bootstrap 用の差分イベントを送信する
                let _ = self
                    .bootstrap_event_tx
                    .send(BootstrapInputEvent::InputRemoved {
                        input_uuid: removed_input.input_uuid.clone(),
                    });

                events.push(TaggedEvent {
                    text: crate::obsws::response::build_input_removed_event(
                        &removed_input.input_name,
                        &removed_input.input_uuid,
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_INPUTS,
                });
                if let Some(scene_items) = scene_items_to_remove {
                    for (scene_name, scene_uuid, scene_item_id) in scene_items {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_scene_item_removed_event(
                                &scene_name,
                                &scene_uuid,
                                scene_item_id,
                                &removed_input.input_name,
                                &removed_input.input_uuid,
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_SCENE_ITEMS,
                        });
                    }
                }
            }
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) async fn handle_set_input_settings(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "SetInputSettings",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let requested_input_lookup =
            match crate::obsws::response::parse_input_lookup_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => Some(fields),
                Err(error) => {
                    return self.build_parse_error_result("SetInputSettings", request_id, &error);
                }
            };
        let execution = crate::obsws::response::execute_set_input_settings(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let response_text = execution.response_text;
        let mut events = Vec::new();
        if execution.request_succeeded
            && let Some((input_uuid, input_name)) = requested_input_lookup
            && let Some(updated_input) = self
                .input_registry
                .find_input(input_uuid.as_deref(), input_name.as_deref())
                .cloned()
        {
            let event = TaggedEvent {
                text: crate::obsws::response::build_input_settings_changed_event(
                    &updated_input.input_name,
                    &updated_input.input_uuid,
                    &updated_input.input.settings,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            };
            // p2p_session にも通知する（chroma key 動的更新用）
            let _ = self.obsws_event_tx.send(event.clone());
            events.push(event);

            // source lifecycle の再評価
            let was_active = self
                .input_source_processors
                .contains_key(&updated_input.input_uuid);
            let is_startable =
                crate::obsws::source::is_source_startable(&updated_input.input.settings);

            match (was_active, is_startable) {
                (false, true) => {
                    // 未起動 → 起動
                    if let Err(e) = self.start_input_source_processor(&updated_input).await {
                        tracing::warn!(
                            "failed to start source processor for input {}: {}",
                            updated_input.input_uuid,
                            e.display()
                        );
                    }
                }
                (true, false) => {
                    // 起動中 → 停止（未起動に戻る）
                    if let Err(e) = self
                        .stop_input_source_processor(&updated_input.input_uuid)
                        .await
                    {
                        tracing::warn!(
                            "failed to stop source processor for input {}: {}",
                            updated_input.input_uuid,
                            e.display()
                        );
                    }
                }
                (true, true) => {
                    // 起動中のまま設定変更 → stop + start で再生成
                    if let Err(e) = self
                        .stop_input_source_processor(&updated_input.input_uuid)
                        .await
                    {
                        tracing::warn!(
                            "failed to stop source processor for input {}: {}",
                            updated_input.input_uuid,
                            e.display()
                        );
                    }
                    if let Err(e) = self.start_input_source_processor(&updated_input).await {
                        tracing::warn!(
                            "failed to restart source processor for input {}: {}",
                            updated_input.input_uuid,
                            e.display()
                        );
                    }
                }
                (false, false) => {
                    // 未起動のまま → 何もしない
                }
            }
        }
        self.build_result_from_response(response_text, events)
    }

    pub(crate) async fn handle_set_input_mute(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::build_set_input_mute_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if execution.request_succeeded
            && let Some(entry) = self.input_registry.find_input(
                execution.input_uuid.as_deref(),
                execution.input_name.as_deref(),
            )
        {
            self.notify_audio_mixer_mute_volume(
                &entry.input_uuid,
                entry.input.input_muted,
                entry.input.input_volume_mul,
            )
            .await;
            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_mute_state_changed_event(
                    &entry.input_name,
                    &entry.input_uuid,
                    entry.input.input_muted,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
        }
        self.build_result_from_response(execution.response_text, events)
    }

    pub(crate) async fn handle_toggle_input_mute(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::build_toggle_input_mute_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if execution.request_succeeded
            && let Some(entry) = self.input_registry.find_input(
                execution.input_uuid.as_deref(),
                execution.input_name.as_deref(),
            )
        {
            self.notify_audio_mixer_mute_volume(
                &entry.input_uuid,
                entry.input.input_muted,
                entry.input.input_volume_mul,
            )
            .await;
            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_mute_state_changed_event(
                    &entry.input_name,
                    &entry.input_uuid,
                    entry.input.input_muted,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
        }
        self.build_result_from_response(execution.response_text, events)
    }

    pub(crate) async fn handle_set_input_volume(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let execution = crate::obsws::response::build_set_input_volume_response(
            request_id,
            request_data,
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if execution.request_succeeded
            && let Some(entry) = self.input_registry.find_input(
                execution.input_uuid.as_deref(),
                execution.input_name.as_deref(),
            )
        {
            self.notify_audio_mixer_mute_volume(
                &entry.input_uuid,
                entry.input.input_muted,
                entry.input.input_volume_mul,
            )
            .await;
            events.push(TaggedEvent {
                text: crate::obsws::response::build_input_volume_changed_event(
                    &entry.input_name,
                    &entry.input_uuid,
                    entry.input.input_volume_db(),
                    entry.input.input_volume_mul.get(),
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            });
        }
        self.build_result_from_response(execution.response_text, events)
    }

    /// audio mixer に入力のミュート・音量設定を通知する
    async fn notify_audio_mixer_mute_volume(
        &self,
        input_uuid: &str,
        muted: bool,
        volume_mul: crate::types::NonNegFiniteF64,
    ) {
        let Some(source_state) = self.input_source_processors.get(input_uuid) else {
            return;
        };
        let Some(audio_track_id) = &source_state.audio_track_id else {
            return;
        };
        let Some(pipeline_handle) = &self.pipeline_handle else {
            return;
        };
        if let Err(e) = crate::mixer::audio::set_track_mute_volume(
            pipeline_handle,
            &self.program_output.audio_mixer_processor_id,
            audio_track_id.clone(),
            muted,
            volume_mul,
        )
        .await
        {
            tracing::warn!("failed to notify audio mixer mute/volume: {}", e.display());
        }
    }

    pub(crate) fn handle_set_input_name(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                "SetInputName",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let requested_input_lookup =
            match crate::obsws::response::parse_input_lookup_fields_for_session(
                request_data.value(),
            ) {
                Ok(fields) => Some(fields),
                Err(error) => {
                    return self.build_parse_error_result("SetInputName", request_id, &error);
                }
            };
        let old_input = requested_input_lookup
            .as_ref()
            .and_then(|(input_uuid, input_name)| {
                self.input_registry
                    .find_input(input_uuid.as_deref(), input_name.as_deref())
                    .cloned()
            });
        let response_text = crate::obsws::response::build_set_input_name_response(
            request_id,
            Some(request_data),
            &mut self.input_registry,
        );
        let mut events = Vec::new();
        if let Some(old_input) = old_input
            && let Some(updated_input) = self
                .input_registry
                .find_input(Some(&old_input.input_uuid), None)
                .cloned()
            && old_input.input_name != updated_input.input_name
        {
            let event = TaggedEvent {
                text: crate::obsws::response::build_input_name_changed_event(
                    &updated_input.input_name,
                    &old_input.input_name,
                    &updated_input.input_uuid,
                ),
                subscription_flag: OBSWS_EVENT_SUB_INPUTS,
            };
            // p2p_session にも通知する（attached_input_name の追従用）
            let _ = self.obsws_event_tx.send(event.clone());
            events.push(event);

            // メディア入力のイベント配信用 input_name も更新する
            if let Some(source_state) = self.input_source_processors.get(&updated_input.input_uuid)
                && let Some(tx) = &source_state.input_name_tx
            {
                let _ = tx.send(updated_input.input_name.clone());
            }
        }
        self.build_result_from_response(response_text, events)
    }

    // -----------------------------------------------------------------------
    // Media Inputs 系ハンドラ
    // -----------------------------------------------------------------------

    pub(crate) fn handle_get_media_input_status(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let (input_uuid, input_name) =
            match crate::obsws::response::parse_request_data_or_error_response(
                "GetMediaInputStatus",
                request_id,
                request_data,
                crate::obsws::response::parse_input_lookup_fields,
            ) {
                Ok(v) => v,
                Err(response) => return self.build_result_from_response(response, Vec::new()),
            };

        let Some(entry) = self
            .input_registry
            .find_input(input_uuid.as_deref(), input_name.as_deref())
        else {
            return self.build_error_result(
                "GetMediaInputStatus",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            );
        };

        // メディア入力（mp4_file_source）のみ対応
        if entry.input.kind_name() != "mp4_file_source" {
            return self.build_error_result(
                "GetMediaInputStatus",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Input is not a media input",
            );
        }

        let Some(source_state) = self.input_source_processors.get(&entry.input_uuid) else {
            // source processor が起動していない場合は None 状態を返す
            let response = crate::obsws::response::build_request_response_success(
                "GetMediaInputStatus",
                request_id,
                |f| {
                    f.member(
                        "mediaState",
                        crate::mp4::reader::MediaPlaybackState::None.as_obs_str(),
                    )?;
                    f.member("mediaDuration", 0i64)?;
                    f.member("mediaCursor", 0i64)
                },
            );
            return self.build_result_from_response(response, Vec::new());
        };

        let (state_str, cursor_ms, duration_ms) = if let Some(handle) = &source_state.media_handle {
            let status = handle.status.borrow();
            (
                status.state.as_obs_str(),
                i64::try_from(status.cursor.as_millis()).unwrap_or(i64::MAX),
                i64::try_from(status.duration.as_millis()).unwrap_or(i64::MAX),
            )
        } else {
            (
                crate::mp4::reader::MediaPlaybackState::None.as_obs_str(),
                0i64,
                0i64,
            )
        };

        let response = crate::obsws::response::build_request_response_success(
            "GetMediaInputStatus",
            request_id,
            |f| {
                f.member("mediaState", state_str)?;
                f.member("mediaDuration", duration_ms)?;
                f.member("mediaCursor", cursor_ms)
            },
        );
        self.build_result_from_response(response, Vec::new())
    }

    pub(crate) fn handle_trigger_media_input_action(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let (input_uuid, input_name, media_action_str) =
            match crate::obsws::response::parse_request_data_or_error_response(
                "TriggerMediaInputAction",
                request_id,
                request_data,
                crate::obsws::response::parse_trigger_media_input_action_fields,
            ) {
                Ok(v) => v,
                Err(response) => return self.build_result_from_response(response, Vec::new()),
            };

        let Some(command) = crate::mp4::reader::MediaInputCommand::from_obs_str(&media_action_str)
        else {
            return self.build_error_result(
                "TriggerMediaInputAction",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Unknown mediaAction value",
            );
        };

        let entry = match self.find_media_input(
            "TriggerMediaInputAction",
            request_id,
            &input_uuid,
            &input_name,
        ) {
            Ok(entry) => entry,
            Err(result) => return *result,
        };

        let handle = match self.get_media_input_handle_or_error(
            "TriggerMediaInputAction",
            request_id,
            &entry.input_uuid,
        ) {
            Ok(handle) => handle,
            Err(result) => return *result,
        };

        // コマンドを送信（バッファが一杯の場合はエラー）
        if handle.command_tx.try_send(command).is_err() {
            return self.build_error_result(
                "TriggerMediaInputAction",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "Failed to send media input command",
            );
        }

        let response = crate::obsws::response::build_request_response_success_no_data(
            "TriggerMediaInputAction",
            request_id,
        );
        self.build_result_from_response(response, Vec::new())
    }

    pub(crate) fn handle_set_media_input_cursor(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let (input_uuid, input_name, cursor_ms) =
            match crate::obsws::response::parse_request_data_or_error_response(
                "SetMediaInputCursor",
                request_id,
                request_data,
                crate::obsws::response::parse_set_media_input_cursor_fields,
            ) {
                Ok(v) => v,
                Err(response) => return self.build_result_from_response(response, Vec::new()),
            };

        self.send_seek_command(
            "SetMediaInputCursor",
            request_id,
            &input_uuid,
            &input_name,
            cursor_ms,
        )
    }

    pub(crate) fn handle_offset_media_input_cursor(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let (input_uuid, input_name, offset_ms) =
            match crate::obsws::response::parse_request_data_or_error_response(
                "OffsetMediaInputCursor",
                request_id,
                request_data,
                crate::obsws::response::parse_offset_media_input_cursor_fields,
            ) {
                Ok(v) => v,
                Err(response) => return self.build_result_from_response(response, Vec::new()),
            };

        // 相対 offset はそのまま reader に渡す（reader 側で現在位置に加算する）
        let entry = match self.find_media_input(
            "OffsetMediaInputCursor",
            request_id,
            &input_uuid,
            &input_name,
        ) {
            Ok(e) => e,
            Err(result) => return *result,
        };

        let handle = match self.get_media_input_handle_or_error(
            "OffsetMediaInputCursor",
            request_id,
            &entry.input_uuid,
        ) {
            Ok(handle) => handle,
            Err(result) => return *result,
        };

        if handle
            .command_tx
            .try_send(crate::mp4::reader::MediaInputCommand::OffsetSeek(offset_ms))
            .is_err()
        {
            return self.build_error_result(
                "OffsetMediaInputCursor",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "Failed to send seek command",
            );
        }

        let response = crate::obsws::response::build_request_response_success_no_data(
            "OffsetMediaInputCursor",
            request_id,
        );
        self.build_result_from_response(response, Vec::new())
    }

    /// メディア入力を検索して mp4_file_source であることを検証する
    fn find_media_input(
        &self,
        request_type: &str,
        request_id: &str,
        input_uuid: &Option<String>,
        input_name: &Option<String>,
    ) -> Result<crate::obsws::input_registry::ObswsInputEntry, Box<CommandResult>> {
        let Some(entry) = self
            .input_registry
            .find_input(input_uuid.as_deref(), input_name.as_deref())
        else {
            return Err(Box::new(self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            )));
        };

        if entry.input.kind_name() != "mp4_file_source" {
            return Err(Box::new(self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Input is not a media input",
            )));
        }

        Ok(entry.clone())
    }

    /// シークコマンドをメディア入力に送信する共通処理
    fn send_seek_command(
        &self,
        request_type: &str,
        request_id: &str,
        input_uuid: &Option<String>,
        input_name: &Option<String>,
        cursor_ms: i64,
    ) -> CommandResult {
        let entry = match self.find_media_input(request_type, request_id, input_uuid, input_name) {
            Ok(e) => e,
            Err(result) => return *result,
        };

        let handle =
            match self.get_media_input_handle_or_error(request_type, request_id, &entry.input_uuid)
            {
                Ok(handle) => handle,
                Err(result) => return *result,
            };

        // 負の値は 0 に clamp する。上限の clamp は reader 側で duration を使って行う
        let clamped_ms = cursor_ms.max(0);
        let position = std::time::Duration::from_millis(
            u64::try_from(clamped_ms).expect("clamped to non-negative"),
        );

        if handle
            .command_tx
            .try_send(crate::mp4::reader::MediaInputCommand::Seek(position))
            .is_err()
        {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "Failed to send seek command",
            );
        }

        let response = crate::obsws::response::build_request_response_success_no_data(
            request_type,
            request_id,
        );
        self.build_result_from_response(response, Vec::new())
    }

    fn get_media_input_handle_or_error<'a>(
        &'a self,
        request_type: &str,
        request_id: &str,
        input_uuid: &str,
    ) -> std::result::Result<&'a crate::mp4::reader::MediaInputHandle, Box<CommandResult>> {
        let handle = self
            .input_source_processors
            .get(input_uuid)
            .and_then(|state| state.media_handle.as_ref());
        handle.ok_or_else(|| {
            Box::new(self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "Media input processor is not available",
            ))
        })
    }

    // -----------------------------------------------------------------------
    // Source processor 管理
    // -----------------------------------------------------------------------

    /// 入力ライフサイクルの source processor を起動する
    pub(crate) async fn start_input_source_processor(
        &mut self,
        input_entry: &crate::obsws::input_registry::ObswsInputEntry,
    ) -> crate::Result<()> {
        let Some(pipeline_handle) = &self.pipeline_handle else {
            return Ok(());
        };
        let mut source_plan = crate::obsws::source::build_record_source_plan(
            input_entry,
            crate::obsws::source::ObswsOutputKind::Program,
            0,
            &input_entry.input_uuid,
            self.input_registry.frame_rate(),
        )
        .map_err(|e| crate::Error::new(format!("failed to build source plan: {}", e.message())))?;

        // mp4_file_source のイベント直接配信用コンテキストを注入する
        let (input_name_tx, input_name_rx) =
            tokio::sync::watch::channel(input_entry.input_name.clone());
        for request in &mut source_plan.requests {
            if let crate::obsws::source::ObswsSourceRequest::CreateMp4FileSource {
                event_ctx, ..
            } = request
            {
                *event_ctx = Some(crate::mp4::reader::MediaEventContext {
                    event_broadcast_tx: self.obsws_event_tx.clone(),
                    input_name_rx: input_name_rx.clone(),
                    input_uuid: input_entry.input_uuid.clone(),
                });
            }
        }

        let mut state = InputSourceState {
            processor_ids: source_plan.source_processor_ids.clone(),
            video_track_id: source_plan.source_video_track_id.clone(),
            audio_track_id: source_plan.source_audio_track_id.clone(),
            media_handle: None,
            input_name_tx: Some(input_name_tx),
        };

        let media_handle = crate::obsws::session::output::start_source_processors(
            pipeline_handle,
            &mut [source_plan],
        )
        .await?;
        state.media_handle = media_handle;

        self.input_source_processors
            .insert(input_entry.input_uuid.clone(), state);
        Ok(())
    }

    /// 入力ライフサイクルの source processor を停止する
    pub(crate) async fn stop_input_source_processor(
        &mut self,
        input_uuid: &str,
    ) -> crate::Result<()> {
        let Some(state) = self.input_source_processors.remove(input_uuid) else {
            return Ok(());
        };
        let Some(pipeline_handle) = &self.pipeline_handle else {
            return Ok(());
        };
        crate::obsws::session::output::stop_source_processors(pipeline_handle, &state.processor_ids)
            .await
    }

    /// 初期入力に対して source processor を一括起動する
    pub async fn start_initial_input_source_processors(&mut self) -> crate::Result<()> {
        let entries: Vec<_> = self
            .input_registry
            .inputs_by_uuid
            .values()
            .cloned()
            .collect();
        for entry in entries {
            if !crate::obsws::source::is_source_startable(&entry.input.settings) {
                continue;
            }
            if let Err(e) = self.start_input_source_processor(&entry).await {
                tracing::warn!(
                    "failed to start source processor for initial input {}: {}",
                    entry.input_uuid,
                    e.display()
                );
            }
        }
        Ok(())
    }

    /// 全入力のミュート・音量を audio mixer に同期する
    pub(crate) async fn sync_all_input_mute_volume(&self) {
        for (input_uuid, source_state) in &self.input_source_processors {
            let Some(ref audio_track_id) = source_state.audio_track_id else {
                continue;
            };
            let Some(entry) = self.input_registry.find_input(Some(input_uuid), None) else {
                continue;
            };
            // デフォルト値（unmuted, 1.0）の場合は通知を省略する
            if !entry.input.input_muted
                && entry.input.input_volume_mul == crate::types::NonNegFiniteF64::ONE
            {
                continue;
            }
            let Some(pipeline_handle) = &self.pipeline_handle else {
                return;
            };
            if let Err(e) = crate::mixer::audio::set_track_mute_volume(
                pipeline_handle,
                &self.program_output.audio_mixer_processor_id,
                audio_track_id.clone(),
                entry.input.input_muted,
                entry.input.input_volume_mul,
            )
            .await
            {
                tracing::warn!(
                    "failed to sync mute/volume for input {input_uuid}: {}",
                    e.display()
                );
            }
        }
    }
}
