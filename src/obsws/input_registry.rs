use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::obsws_protocol::OBSWS_DEFAULT_SCENE_NAME;

mod scene_item;
#[cfg(test)]
#[path = "input_registry/tests.rs"]
mod tests;
mod types;

pub(crate) use self::types::*;

impl ObswsInputRegistry {
    pub fn new(
        record_directory: PathBuf,
        canvas_width: crate::types::EvenUsize,
        canvas_height: crate::types::EvenUsize,
        frame_rate: crate::video::FrameRate,
    ) -> Self {
        let mut scenes_by_name = BTreeMap::new();
        scenes_by_name.insert(
            OBSWS_DEFAULT_SCENE_NAME.to_owned(),
            ObswsSceneState {
                scene_uuid: "10000000-0000-0000-0000-000000000000".to_owned(),
                items: Vec::new(),
            },
        );
        Self {
            inputs_by_uuid: BTreeMap::new(),
            uuids_by_name: BTreeMap::new(),
            scenes_by_name,
            scene_order: vec![OBSWS_DEFAULT_SCENE_NAME.to_owned()],
            current_program_scene_name: OBSWS_DEFAULT_SCENE_NAME.to_owned(),
            current_preview_scene_name: OBSWS_DEFAULT_SCENE_NAME.to_owned(),
            scene_transition_overrides: BTreeMap::new(),
            next_input_id: 0,
            next_scene_id: 1,
            next_scene_item_id: 1,
            next_stream_run_id: 0,
            next_record_run_id: 0,
            stream_service_settings: ObswsStreamServiceSettings::default(),
            transition_runtime: ObswsTransitionRuntimeState::default(),
            stream_runtime: ObswsStreamRuntimeState::default(),
            rtmp_outbound_settings: ObswsRtmpOutboundSettings::default(),
            rtmp_outbound_runtime: ObswsRtmpOutboundRuntimeState::default(),
            next_rtmp_outbound_run_id: 0,
            record_directory,
            record_runtime: ObswsRecordRuntimeState::default(),
            canvas_width,
            canvas_height,
            frame_rate,
        }
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self::new(
            PathBuf::from("recordings-for-test"),
            crate::types::EvenUsize::new(1920).unwrap(),
            crate::types::EvenUsize::new(1080).unwrap(),
            crate::video::FrameRate::FPS_30,
        )
    }

    pub fn list_inputs(&self) -> Vec<ObswsInputEntry> {
        self.inputs_by_uuid.values().cloned().collect()
    }

    pub fn supported_input_kinds(&self) -> &'static [&'static str] {
        &OBSWS_SUPPORTED_INPUT_KINDS
    }

    pub fn create_input(
        &mut self,
        scene_name: &str,
        input_name: &str,
        input: ObswsInput,
        scene_item_enabled: bool,
    ) -> Result<(ObswsInputEntry, i64), CreateInputError> {
        if !self.scenes_by_name.contains_key(scene_name) {
            return Err(CreateInputError::UnsupportedSceneName);
        }
        if self.uuids_by_name.contains_key(input_name) {
            return Err(CreateInputError::InputNameAlreadyExists);
        }

        let input_uuid = self
            .next_input_uuid()
            .map_err(|_| CreateInputError::InputIdOverflow)?;
        let entry = ObswsInputEntry {
            input_uuid: input_uuid.clone(),
            input_name: input_name.to_owned(),
            input,
        };
        self.uuids_by_name
            .insert(entry.input_name.clone(), input_uuid);
        self.inputs_by_uuid
            .insert(entry.input_uuid.clone(), entry.clone());
        let scene_item_id = self
            .next_scene_item_id()
            .map_err(|_| CreateInputError::InputIdOverflow)?;
        let scene = self
            .scenes_by_name
            .get_mut(scene_name)
            .expect("BUG: scene must exist after validation");
        scene.items.push(ObswsSceneItemState {
            scene_item_id,
            input_uuid: entry.input_uuid.clone(),
            enabled: scene_item_enabled,
            locked: false,
            blend_mode: ObswsSceneItemBlendMode::default(),
            transform: ObswsSceneItemTransform::default(),
        });

        Ok((entry, scene_item_id))
    }

    pub fn create_scene(&mut self, scene_name: &str) -> Result<ObswsSceneEntry, CreateSceneError> {
        if self.scenes_by_name.contains_key(scene_name) {
            return Err(CreateSceneError::SceneNameAlreadyExists);
        }
        let scene_id = self.next_scene_id;
        if scene_id > OBSWS_MAX_SCENE_ID_FOR_UUID_SUFFIX {
            return Err(CreateSceneError::SceneIdOverflow);
        }
        self.next_scene_id = self
            .next_scene_id
            .checked_add(1)
            .ok_or(CreateSceneError::SceneIdOverflow)?;
        let scene_uuid = format!("10000000-0000-0000-0000-{scene_id:012x}");

        self.scenes_by_name.insert(
            scene_name.to_owned(),
            ObswsSceneState {
                scene_uuid: scene_uuid.clone(),
                items: Vec::new(),
            },
        );
        self.scene_order.push(scene_name.to_owned());
        Ok(ObswsSceneEntry {
            scene_index: self.scene_order.len().saturating_sub(1),
            scene_name: scene_name.to_owned(),
            scene_uuid,
        })
    }

    pub fn set_scene_name(
        &mut self,
        scene_name: &str,
        new_scene_name: &str,
    ) -> Result<ObswsSceneEntry, SetSceneNameError> {
        if !self.scenes_by_name.contains_key(scene_name) {
            return Err(SetSceneNameError::SceneNotFound);
        }
        if scene_name != new_scene_name && self.scenes_by_name.contains_key(new_scene_name) {
            return Err(SetSceneNameError::SceneNameAlreadyExists);
        }
        if scene_name == new_scene_name {
            let scene_index = self
                .scene_order
                .iter()
                .position(|name| name == scene_name)
                .expect("BUG: scene order must contain the scene");
            let scene_uuid = self
                .scenes_by_name
                .get(scene_name)
                .map(|scene| scene.scene_uuid.clone())
                .expect("BUG: scene must exist");
            return Ok(ObswsSceneEntry {
                scene_index,
                scene_name: scene_name.to_owned(),
                scene_uuid,
            });
        }

        let scene = self
            .scenes_by_name
            .remove(scene_name)
            .expect("BUG: scene must exist after validation");
        self.scenes_by_name.insert(new_scene_name.to_owned(), scene);

        let scene_index = self
            .scene_order
            .iter()
            .position(|name| name == scene_name)
            .expect("BUG: scene order must contain the scene");
        self.scene_order[scene_index] = new_scene_name.to_owned();

        if let Some(override_entry) = self.scene_transition_overrides.remove(scene_name) {
            self.scene_transition_overrides
                .insert(new_scene_name.to_owned(), override_entry);
        }

        if self.current_program_scene_name == scene_name {
            self.current_program_scene_name = new_scene_name.to_owned();
        }
        if self.current_preview_scene_name == scene_name {
            self.current_preview_scene_name = new_scene_name.to_owned();
        }

        let scene_uuid = self
            .scenes_by_name
            .get(new_scene_name)
            .map(|scene| scene.scene_uuid.clone())
            .expect("BUG: renamed scene must exist");
        Ok(ObswsSceneEntry {
            scene_index,
            scene_name: new_scene_name.to_owned(),
            scene_uuid,
        })
    }

    pub fn remove_scene(&mut self, scene_name: &str) -> Result<ObswsSceneEntry, RemoveSceneError> {
        if !self.scenes_by_name.contains_key(scene_name) {
            return Err(RemoveSceneError::SceneNotFound);
        }
        if self.scene_order.len() <= 1 {
            return Err(RemoveSceneError::LastSceneNotRemovable);
        }

        let Some(scene_index) = self.scene_order.iter().position(|name| name == scene_name) else {
            return Err(RemoveSceneError::SceneNotFound);
        };
        let scene_uuid = self
            .scenes_by_name
            .remove(scene_name)
            .map(|scene| scene.scene_uuid)
            .ok_or(RemoveSceneError::SceneNotFound)?;
        self.scene_order.retain(|name| name != scene_name);

        if self.current_program_scene_name == scene_name {
            let new_scene_name = self
                .scene_order
                .first()
                .expect("infallible: at least one scene remains after scene deletion")
                .clone();
            self.current_program_scene_name = new_scene_name;
        }
        if self.current_preview_scene_name == scene_name {
            let new_scene_name = self
                .scene_order
                .first()
                .expect("infallible: at least one scene remains after scene deletion")
                .clone();
            self.current_preview_scene_name = new_scene_name;
        }

        Ok(ObswsSceneEntry {
            scene_index,
            scene_name: scene_name.to_owned(),
            scene_uuid,
        })
    }

    pub fn list_scenes(&self) -> Vec<ObswsSceneEntry> {
        // OBS は sceneIndex 降順（後から作ったシーンが先頭）で返すため逆順にする
        let mut scenes: Vec<ObswsSceneEntry> = self
            .scene_order
            .iter()
            .enumerate()
            .filter_map(|(index, scene_name)| {
                self.scenes_by_name
                    .get(scene_name)
                    .map(|scene| ObswsSceneEntry {
                        scene_index: index,
                        scene_name: scene_name.clone(),
                        scene_uuid: scene.scene_uuid.clone(),
                    })
            })
            .collect();
        scenes.reverse();
        scenes
    }

    pub fn current_program_scene(&self) -> Option<ObswsSceneEntry> {
        let scene_name = &self.current_program_scene_name;
        let scene = self.scenes_by_name.get(scene_name)?;
        let scene_index = self
            .scene_order
            .iter()
            .position(|name| name == scene_name)?;
        Some(ObswsSceneEntry {
            scene_index,
            scene_name: scene_name.clone(),
            scene_uuid: scene.scene_uuid.clone(),
        })
    }

    pub fn set_current_program_scene(
        &mut self,
        scene_name: &str,
    ) -> Result<(), SetCurrentProgramSceneError> {
        if !self.scenes_by_name.contains_key(scene_name) {
            return Err(SetCurrentProgramSceneError::SceneNotFound);
        }
        self.current_program_scene_name = scene_name.to_owned();
        Ok(())
    }

    pub fn current_preview_scene(&self) -> Option<ObswsSceneEntry> {
        let scene_name = &self.current_preview_scene_name;
        let scene = self.scenes_by_name.get(scene_name)?;
        let scene_index = self
            .scene_order
            .iter()
            .position(|name| name == scene_name)?;
        Some(ObswsSceneEntry {
            scene_index,
            scene_name: scene_name.clone(),
            scene_uuid: scene.scene_uuid.clone(),
        })
    }

    pub fn set_current_preview_scene(
        &mut self,
        scene_name: &str,
    ) -> Result<(), SetCurrentPreviewSceneError> {
        if !self.scenes_by_name.contains_key(scene_name) {
            return Err(SetCurrentPreviewSceneError::SceneNotFound);
        }
        self.current_preview_scene_name = scene_name.to_owned();
        Ok(())
    }

    pub fn is_source_active(
        &self,
        input_uuid: Option<&str>,
        input_name: Option<&str>,
    ) -> Result<bool, GetSourceActiveError> {
        let input = self
            .find_input(input_uuid, input_name)
            .ok_or(GetSourceActiveError::SourceNotFound)?;
        let Some(scene) = self.scenes_by_name.get(&self.current_program_scene_name) else {
            return Ok(false);
        };
        Ok(scene
            .items
            .iter()
            .any(|item| item.enabled && item.input_uuid == input.input_uuid))
    }

    pub fn get_scene_transition_override(
        &self,
        scene_name: &str,
    ) -> Result<ObswsSceneTransitionOverride, GetSceneSceneTransitionOverrideError> {
        if !self.scenes_by_name.contains_key(scene_name) {
            return Err(GetSceneSceneTransitionOverrideError::SceneNotFound);
        }
        Ok(self
            .scene_transition_overrides
            .get(scene_name)
            .cloned()
            .unwrap_or_default())
    }

    pub fn set_scene_transition_override(
        &mut self,
        scene_name: &str,
        transition_name: Option<&str>,
        transition_duration: Option<i64>,
    ) -> Result<ObswsSceneTransitionOverride, SetSceneSceneTransitionOverrideError> {
        if !self.scenes_by_name.contains_key(scene_name) {
            return Err(SetSceneSceneTransitionOverrideError::SceneNotFound);
        }
        if let Some(transition_name) = transition_name
            && !OBSWS_SUPPORTED_TRANSITION_KINDS.contains(&transition_name)
        {
            return Err(SetSceneSceneTransitionOverrideError::TransitionNotFound);
        }
        if let Some(transition_duration) = transition_duration
            && !(OBSWS_MIN_TRANSITION_DURATION_MS..=OBSWS_MAX_TRANSITION_DURATION_MS)
                .contains(&transition_duration)
        {
            return Err(SetSceneSceneTransitionOverrideError::InvalidTransitionDuration);
        }

        if transition_name.is_none() && transition_duration.is_none() {
            self.scene_transition_overrides.remove(scene_name);
            return Ok(ObswsSceneTransitionOverride::default());
        }

        let entry = self
            .scene_transition_overrides
            .entry(scene_name.to_owned())
            .or_default();
        entry.transition_name = transition_name.map(ToOwned::to_owned);
        entry.transition_duration = transition_duration;
        Ok(entry.clone())
    }

    pub fn supported_transition_kinds(&self) -> &'static [&'static str] {
        &OBSWS_SUPPORTED_TRANSITION_KINDS
    }

    pub fn current_scene_transition_name(&self) -> &str {
        &self.transition_runtime.current_transition_name
    }

    pub fn current_scene_transition_duration_ms(&self) -> i64 {
        self.transition_runtime.current_transition_duration_ms
    }

    pub fn current_scene_transition_settings(&self) -> &nojson::RawJsonOwned {
        &self.transition_runtime.current_transition_settings
    }

    pub fn current_tbar_position(&self) -> f64 {
        self.transition_runtime.current_tbar_position
    }

    pub fn set_current_scene_transition(
        &mut self,
        transition_name: &str,
    ) -> Result<(), SetCurrentSceneTransitionError> {
        if !OBSWS_SUPPORTED_TRANSITION_KINDS.contains(&transition_name) {
            return Err(SetCurrentSceneTransitionError::TransitionNotFound);
        }
        self.transition_runtime.current_transition_name = transition_name.to_owned();
        Ok(())
    }

    pub fn set_current_scene_transition_duration_ms(
        &mut self,
        transition_duration_ms: i64,
    ) -> Result<(), SetCurrentSceneTransitionDurationError> {
        if !(OBSWS_MIN_TRANSITION_DURATION_MS..=OBSWS_MAX_TRANSITION_DURATION_MS)
            .contains(&transition_duration_ms)
        {
            return Err(SetCurrentSceneTransitionDurationError::InvalidTransitionDuration);
        }
        self.transition_runtime.current_transition_duration_ms = transition_duration_ms;
        Ok(())
    }

    pub fn set_current_scene_transition_settings(
        &mut self,
        transition_settings: nojson::RawJsonOwned,
    ) -> Result<(), SetCurrentSceneTransitionSettingsError> {
        if transition_settings.value().kind() != nojson::JsonValueKind::Object {
            return Err(SetCurrentSceneTransitionSettingsError::InvalidTransitionSettings);
        }
        self.transition_runtime.current_transition_settings = transition_settings;
        Ok(())
    }

    pub fn set_tbar_position(&mut self, tbar_position: f64) -> Result<(), SetTBarPositionError> {
        if !tbar_position.is_finite()
            || !(OBSWS_MIN_TBAR_POSITION..=OBSWS_MAX_TBAR_POSITION).contains(&tbar_position)
        {
            return Err(SetTBarPositionError::InvalidTBarPosition);
        }
        self.transition_runtime.current_tbar_position = tbar_position;
        Ok(())
    }

    pub fn list_current_program_scene_inputs(&self) -> Vec<ObswsInputEntry> {
        let Some(scene) = self.scenes_by_name.get(&self.current_program_scene_name) else {
            return Vec::new();
        };
        scene
            .items
            .iter()
            .filter(|item| item.enabled)
            .filter_map(|item| self.inputs_by_uuid.get(&item.input_uuid).cloned())
            .collect()
    }

    pub fn list_current_program_scene_input_entries(&self) -> Vec<ObswsSceneInputEntry> {
        let Some(scene) = self.scenes_by_name.get(&self.current_program_scene_name) else {
            return Vec::new();
        };
        scene
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.enabled)
            .filter_map(|(index, item)| {
                let input = self.inputs_by_uuid.get(&item.input_uuid)?.clone();
                Some(ObswsSceneInputEntry {
                    input,
                    scene_item_index: index,
                    transform: item.transform.clone(),
                })
            })
            .collect()
    }

    pub fn canvas_width(&self) -> crate::types::EvenUsize {
        self.canvas_width
    }

    pub fn canvas_height(&self) -> crate::types::EvenUsize {
        self.canvas_height
    }

    pub fn frame_rate(&self) -> crate::video::FrameRate {
        self.frame_rate
    }

    pub fn resolve_scene_name(
        &self,
        scene_name: Option<&str>,
        scene_uuid: Option<&str>,
    ) -> Option<String> {
        if let Some(scene_name) = scene_name {
            if self.scenes_by_name.contains_key(scene_name) {
                return Some(scene_name.to_owned());
            }
            return None;
        }

        let scene_uuid = scene_uuid?;
        self.scenes_by_name
            .iter()
            .find(|(_, scene)| scene.scene_uuid == scene_uuid)
            .map(|(scene_name, _)| scene_name.clone())
    }

    pub fn get_scene_uuid(&self, scene_name: &str) -> Option<String> {
        self.scenes_by_name
            .get(scene_name)
            .map(|scene| scene.scene_uuid.clone())
    }

    pub fn stream_service_settings(&self) -> &ObswsStreamServiceSettings {
        &self.stream_service_settings
    }

    pub fn next_stream_run_id(&mut self) -> Result<u64, RunIdOverflowError> {
        let run_id = self.next_stream_run_id;
        self.next_stream_run_id = self
            .next_stream_run_id
            .checked_add(1)
            .ok_or(RunIdOverflowError::Stream)?;
        Ok(run_id)
    }

    pub fn next_record_run_id(&mut self) -> Result<u64, RunIdOverflowError> {
        let run_id = self.next_record_run_id;
        self.next_record_run_id = self
            .next_record_run_id
            .checked_add(1)
            .ok_or(RunIdOverflowError::Record)?;
        Ok(run_id)
    }

    pub fn set_stream_service_settings(&mut self, settings: ObswsStreamServiceSettings) {
        self.stream_service_settings = settings;
    }

    pub fn activate_stream(&mut self, run: ObswsStreamRun) -> Result<(), ActivateStreamError> {
        if self.stream_runtime.active {
            return Err(ActivateStreamError::AlreadyActive);
        }
        self.stream_runtime.active = true;
        self.stream_runtime.started_at = Some(Instant::now());
        self.stream_runtime.run = Some(run);
        Ok(())
    }

    pub fn deactivate_stream(&mut self) -> Option<ObswsStreamRun> {
        let run = self.stream_runtime.run.take();
        self.stream_runtime.active = false;
        self.stream_runtime.started_at = None;
        run
    }

    pub fn is_stream_active(&self) -> bool {
        self.stream_runtime.active
    }

    pub fn stream_run(&self) -> Option<ObswsStreamRun> {
        self.stream_runtime.run.clone()
    }

    pub fn stream_uptime(&self) -> Duration {
        self.stream_runtime
            .started_at
            .map(|started_at| started_at.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    pub fn rtmp_outbound_settings(&self) -> &ObswsRtmpOutboundSettings {
        &self.rtmp_outbound_settings
    }

    pub fn set_rtmp_outbound_settings(&mut self, settings: ObswsRtmpOutboundSettings) {
        self.rtmp_outbound_settings = settings;
    }

    pub fn next_rtmp_outbound_run_id(&mut self) -> Result<u64, RunIdOverflowError> {
        let run_id = self.next_rtmp_outbound_run_id;
        self.next_rtmp_outbound_run_id = self
            .next_rtmp_outbound_run_id
            .checked_add(1)
            .ok_or(RunIdOverflowError::RtmpOutbound)?;
        Ok(run_id)
    }

    pub fn activate_rtmp_outbound(
        &mut self,
        run: ObswsRtmpOutboundRun,
    ) -> Result<(), ActivateRtmpOutboundError> {
        if self.rtmp_outbound_runtime.active {
            return Err(ActivateRtmpOutboundError::AlreadyActive);
        }
        self.rtmp_outbound_runtime.active = true;
        self.rtmp_outbound_runtime.started_at = Some(Instant::now());
        self.rtmp_outbound_runtime.run = Some(run);
        Ok(())
    }

    pub fn deactivate_rtmp_outbound(&mut self) -> Option<ObswsRtmpOutboundRun> {
        let run = self.rtmp_outbound_runtime.run.take();
        self.rtmp_outbound_runtime.active = false;
        self.rtmp_outbound_runtime.started_at = None;
        run
    }

    pub fn is_rtmp_outbound_active(&self) -> bool {
        self.rtmp_outbound_runtime.active
    }

    pub fn rtmp_outbound_run(&self) -> Option<ObswsRtmpOutboundRun> {
        self.rtmp_outbound_runtime.run.clone()
    }

    pub fn rtmp_outbound_uptime(&self) -> Duration {
        self.rtmp_outbound_runtime
            .started_at
            .map(|started_at| started_at.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    pub fn record_directory(&self) -> &Path {
        &self.record_directory
    }

    pub fn set_record_directory(&mut self, record_directory: PathBuf) {
        self.record_directory = record_directory;
    }

    pub fn activate_record(&mut self, run: ObswsRecordRun) -> Result<(), ActivateRecordError> {
        if self.record_runtime.active {
            return Err(ActivateRecordError::AlreadyActive);
        }
        self.record_runtime.active = true;
        self.record_runtime.started_at = Some(Instant::now());
        self.record_runtime.run = Some(run);
        Ok(())
    }

    pub fn deactivate_record(&mut self) -> Option<ObswsRecordRun> {
        let run = self.record_runtime.run.take();
        self.record_runtime.active = false;
        self.record_runtime.started_at = None;
        run
    }

    pub fn is_record_active(&self) -> bool {
        self.record_runtime.active
    }

    pub fn record_run(&self) -> Option<ObswsRecordRun> {
        self.record_runtime.run.clone()
    }

    pub fn record_uptime(&self) -> Duration {
        self.record_runtime
            .started_at
            .map(|started_at| started_at.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    pub fn record_output_path(&self) -> Option<&Path> {
        self.record_runtime
            .run
            .as_ref()
            .map(|run| run.output_path.as_path())
    }

    pub fn remove_input(
        &mut self,
        input_uuid: Option<&str>,
        input_name: Option<&str>,
    ) -> Option<ObswsInputEntry> {
        if let Some(input_uuid) = input_uuid {
            let removed = self.inputs_by_uuid.remove(input_uuid)?;
            self.uuids_by_name.remove(&removed.input_name);
            for scene in self.scenes_by_name.values_mut() {
                scene
                    .items
                    .retain(|item| item.input_uuid != removed.input_uuid);
            }
            return Some(removed);
        }

        let input_name = input_name?;
        let input_uuid = self.uuids_by_name.remove(input_name)?;
        let removed = self.inputs_by_uuid.remove(&input_uuid)?;
        for scene in self.scenes_by_name.values_mut() {
            scene
                .items
                .retain(|item| item.input_uuid != removed.input_uuid);
        }
        Some(removed)
    }

    pub fn set_input_settings(
        &mut self,
        input_uuid: Option<&str>,
        input_name: Option<&str>,
        input_settings: nojson::RawJsonValue<'_, '_>,
        overlay: bool,
    ) -> Result<(), SetInputSettingsError> {
        let target_input_uuid = if let Some(input_uuid) = input_uuid {
            input_uuid.to_owned()
        } else {
            let Some(input_name) = input_name else {
                return Err(SetInputSettingsError::InputNotFound);
            };
            let Some(input_uuid) = self.uuids_by_name.get(input_name) else {
                return Err(SetInputSettingsError::InputNotFound);
            };
            input_uuid.clone()
        };

        let Some(input_entry) = self.inputs_by_uuid.get_mut(&target_input_uuid) else {
            return Err(SetInputSettingsError::InputNotFound);
        };

        let settings_result = if overlay {
            input_entry
                .input
                .settings
                .overlay_with_settings(input_settings)
        } else {
            ObswsInputSettings::from_kind_and_settings(
                input_entry.input.kind_name(),
                input_settings,
            )
        };
        let settings = settings_result.map_err(|e| match e {
            ParseInputSettingsError::InvalidInputSettings(message) => {
                SetInputSettingsError::InvalidInputSettings(message)
            }
            ParseInputSettingsError::UnsupportedInputKind => {
                SetInputSettingsError::InvalidInputSettings(
                    "Unsupported input kind for inputSettings update".to_owned(),
                )
            }
        })?;
        input_entry.input.settings = settings;
        Ok(())
    }

    pub fn set_input_name(
        &mut self,
        input_uuid: Option<&str>,
        input_name: Option<&str>,
        new_input_name: &str,
    ) -> Result<(), SetInputNameError> {
        let target_input_uuid = if let Some(input_uuid) = input_uuid {
            input_uuid.to_owned()
        } else {
            let Some(input_name) = input_name else {
                return Err(SetInputNameError::InputNotFound);
            };
            let Some(input_uuid) = self.uuids_by_name.get(input_name) else {
                return Err(SetInputNameError::InputNotFound);
            };
            input_uuid.clone()
        };

        if let Some(existing_input_uuid) = self.uuids_by_name.get(new_input_name)
            && existing_input_uuid != &target_input_uuid
        {
            return Err(SetInputNameError::InputNameAlreadyExists);
        }

        let Some(input_entry) = self.inputs_by_uuid.get_mut(&target_input_uuid) else {
            return Err(SetInputNameError::InputNotFound);
        };
        if input_entry.input_name == new_input_name {
            return Ok(());
        }

        let old_input_name =
            std::mem::replace(&mut input_entry.input_name, new_input_name.to_owned());
        self.uuids_by_name.remove(&old_input_name);
        self.uuids_by_name
            .insert(new_input_name.to_owned(), target_input_uuid);
        Ok(())
    }

    pub fn get_input_default_settings(
        &self,
        input_kind: &str,
    ) -> Result<ObswsInputSettings, ParseInputSettingsError> {
        ObswsInputSettings::default_for_kind(input_kind)
    }

    pub fn find_input(
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
    pub fn insert_for_test(&mut self, entry: ObswsInputEntry) {
        self.uuids_by_name
            .insert(entry.input_name.clone(), entry.input_uuid.clone());
        self.inputs_by_uuid.insert(entry.input_uuid.clone(), entry);
    }

    fn next_input_uuid(&mut self) -> Result<String, InputIdOverflowError> {
        let input_id = self.next_input_id;
        if input_id > OBSWS_MAX_INPUT_ID_FOR_UUID_SUFFIX {
            return Err(InputIdOverflowError);
        }
        self.next_input_id = self
            .next_input_id
            .checked_add(1)
            .ok_or(InputIdOverflowError)?;
        Ok(format!("00000000-0000-0000-0000-{input_id:012x}"))
    }

    fn next_scene_item_id(&mut self) -> Result<i64, SceneItemIdOverflowError> {
        let scene_item_id = self.next_scene_item_id;
        self.next_scene_item_id = self
            .next_scene_item_id
            .checked_add(1)
            .ok_or(SceneItemIdOverflowError)?;
        Ok(scene_item_id)
    }

    fn build_scene_item_entries(&self, scene: &ObswsSceneState) -> Vec<ObswsSceneItemEntry> {
        scene
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let input_entry = self.inputs_by_uuid.get(&item.input_uuid)?;
                Some(ObswsSceneItemEntry {
                    scene_item_id: item.scene_item_id,
                    source_name: input_entry.input_name.clone(),
                    source_uuid: input_entry.input_uuid.clone(),
                    input_kind: input_entry.input.kind_name().to_owned(),
                    source_type: "OBS_SOURCE_TYPE_INPUT".to_owned(),
                    scene_item_enabled: item.enabled,
                    scene_item_locked: item.locked,
                    scene_item_blend_mode: item.blend_mode.as_str().to_owned(),
                    scene_item_index: index as i64,
                    scene_item_transform: item.transform.clone(),
                    is_group: None,
                })
            })
            .collect()
    }

    fn find_scene_item(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<&ObswsSceneItemState, SceneItemLookupError> {
        let scene = self
            .scenes_by_name
            .get(scene_name)
            .ok_or(SceneItemLookupError::SceneNotFound)?;
        scene
            .items
            .iter()
            .find(|item| item.scene_item_id == scene_item_id)
            .ok_or(SceneItemLookupError::SceneItemNotFound)
    }

    fn find_scene_item_mut(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<&mut ObswsSceneItemState, SceneItemLookupError> {
        let scene = self
            .scenes_by_name
            .get_mut(scene_name)
            .ok_or(SceneItemLookupError::SceneNotFound)?;
        scene
            .items
            .iter_mut()
            .find(|item| item.scene_item_id == scene_item_id)
            .ok_or(SceneItemLookupError::SceneItemNotFound)
    }
}

fn apply_transform_patch_value<T: PartialEq>(changed: &mut bool, dst: &mut T, value: Option<T>) {
    let Some(value) = value else {
        return;
    };
    if *dst != value {
        *changed = true;
        *dst = value;
    }
}
