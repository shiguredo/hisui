use super::*;

impl ObswsInputRegistry {
    pub fn get_scene_item_id(
        &self,
        scene_name: &str,
        source_name: &str,
        search_offset: i64,
    ) -> Result<i64, GetSceneItemIdError> {
        if search_offset != 0 {
            return Err(GetSceneItemIdError::SearchOffsetUnsupported);
        }
        let Some(scene) = self.scenes_by_name.get(scene_name) else {
            return Err(GetSceneItemIdError::SceneNotFound);
        };
        let Some(scene_item_id) = scene.items.iter().find_map(|item| {
            let input = self.inputs_by_uuid.get(&item.input_uuid)?;
            (input.input_name == source_name).then_some(item.scene_item_id)
        }) else {
            return Err(GetSceneItemIdError::SourceNotFound);
        };
        Ok(scene_item_id)
    }

    pub fn list_scene_items(
        &self,
        scene_name: &str,
    ) -> Result<Vec<ObswsSceneItemEntry>, GetSceneItemListError> {
        let Some(scene) = self.scenes_by_name.get(scene_name) else {
            return Err(GetSceneItemListError::SceneNotFound);
        };
        Ok(self.build_scene_item_entries(scene))
    }

    pub fn create_scene_item(
        &mut self,
        scene_name: &str,
        source_uuid: Option<&str>,
        source_name: Option<&str>,
        scene_item_enabled: bool,
    ) -> Result<ObswsSceneItemRef, CreateSceneItemError> {
        let input_entry = self
            .find_input(source_uuid, source_name)
            .cloned()
            .ok_or(CreateSceneItemError::SourceNotFound)?;
        let scene_item_id = self
            .next_scene_item_id()
            .map_err(|_| CreateSceneItemError::SceneItemIdOverflow)?;
        let scene = self
            .scenes_by_name
            .get_mut(scene_name)
            .ok_or(CreateSceneItemError::SceneNotFound)?;
        scene.items.push(ObswsSceneItemState {
            scene_item_id,
            input_uuid: input_entry.input_uuid.clone(),
            enabled: scene_item_enabled,
            locked: false,
            blend_mode: ObswsSceneItemBlendMode::default(),
            transform: ObswsSceneItemTransform::default(),
        });
        let scene_item_index = scene.items.len().saturating_sub(1) as i64;
        Ok(ObswsSceneItemRef {
            scene_name: scene_name.to_owned(),
            scene_uuid: scene.scene_uuid.clone(),
            scene_item: ObswsSceneItemEntry {
                scene_item_id,
                source_name: input_entry.input_name,
                source_uuid: input_entry.input_uuid,
                scene_item_enabled,
                scene_item_locked: false,
                scene_item_blend_mode: ObswsSceneItemBlendMode::default().as_str().to_owned(),
                scene_item_index,
                is_group: false,
            },
        })
    }

    pub fn remove_scene_item(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<ObswsSceneItemRef, SceneItemLookupError> {
        let scene = self
            .scenes_by_name
            .get_mut(scene_name)
            .ok_or(SceneItemLookupError::SceneNotFound)?;
        let Some(position) = scene
            .items
            .iter()
            .position(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(SceneItemLookupError::SceneItemNotFound);
        };
        let removed = scene.items.remove(position);
        let scene_uuid = scene.scene_uuid.clone();
        let input_entry = self
            .inputs_by_uuid
            .get(&removed.input_uuid)
            .ok_or(SceneItemLookupError::SceneItemNotFound)?;
        Ok(ObswsSceneItemRef {
            scene_name: scene_name.to_owned(),
            scene_uuid,
            scene_item: ObswsSceneItemEntry {
                scene_item_id: removed.scene_item_id,
                source_name: input_entry.input_name.clone(),
                source_uuid: input_entry.input_uuid.clone(),
                scene_item_enabled: removed.enabled,
                scene_item_locked: removed.locked,
                scene_item_blend_mode: removed.blend_mode.as_str().to_owned(),
                scene_item_index: position as i64,
                is_group: false,
            },
        })
    }

    pub fn duplicate_scene_item(
        &mut self,
        from_scene_name: &str,
        to_scene_name: &str,
        scene_item_id: i64,
    ) -> Result<ObswsSceneItemRef, DuplicateSceneItemError> {
        let (input_uuid, enabled, locked, blend_mode, transform) = {
            let from_scene = self
                .scenes_by_name
                .get(from_scene_name)
                .ok_or(DuplicateSceneItemError::SourceScene)?;
            let Some(from_item) = from_scene
                .items
                .iter()
                .find(|item| item.scene_item_id == scene_item_id)
            else {
                return Err(DuplicateSceneItemError::SourceSceneItem);
            };
            (
                from_item.input_uuid.clone(),
                from_item.enabled,
                from_item.locked,
                from_item.blend_mode,
                from_item.transform.clone(),
            )
        };
        let input_entry = self
            .inputs_by_uuid
            .get(&input_uuid)
            .ok_or(DuplicateSceneItemError::SourceSceneItem)?
            .clone();
        let new_scene_item_id = self
            .next_scene_item_id()
            .map_err(|_| DuplicateSceneItemError::SceneItemIdOverflow)?;
        let to_scene = self
            .scenes_by_name
            .get_mut(to_scene_name)
            .ok_or(DuplicateSceneItemError::DestinationScene)?;
        to_scene.items.push(ObswsSceneItemState {
            scene_item_id: new_scene_item_id,
            input_uuid: input_uuid.clone(),
            enabled,
            locked,
            blend_mode,
            transform,
        });
        let scene_item_index = to_scene.items.len().saturating_sub(1) as i64;
        Ok(ObswsSceneItemRef {
            scene_name: to_scene_name.to_owned(),
            scene_uuid: to_scene.scene_uuid.clone(),
            scene_item: ObswsSceneItemEntry {
                scene_item_id: new_scene_item_id,
                source_name: input_entry.input_name,
                source_uuid: input_entry.input_uuid,
                scene_item_enabled: enabled,
                scene_item_locked: locked,
                scene_item_blend_mode: blend_mode.as_str().to_owned(),
                scene_item_index,
                is_group: false,
            },
        })
    }

    pub fn get_scene_item_source(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<(String, String), SceneItemLookupError> {
        let scene_item = self.find_scene_item(scene_name, scene_item_id)?;
        let input_entry = self
            .inputs_by_uuid
            .get(&scene_item.input_uuid)
            .ok_or(SceneItemLookupError::SceneItemNotFound)?;
        Ok((
            input_entry.input_name.clone(),
            input_entry.input_uuid.clone(),
        ))
    }

    pub fn get_scene_item_index(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<i64, SceneItemLookupError> {
        let scene = self
            .scenes_by_name
            .get(scene_name)
            .ok_or(SceneItemLookupError::SceneNotFound)?;
        let Some(index) = scene
            .items
            .iter()
            .position(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(SceneItemLookupError::SceneItemNotFound);
        };
        Ok(index as i64)
    }

    pub fn set_scene_item_index(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
        scene_item_index: i64,
    ) -> Result<SetSceneItemIndexResult, SetSceneItemIndexError> {
        let scene = self
            .scenes_by_name
            .get_mut(scene_name)
            .ok_or(SetSceneItemIndexError::SceneNotFound)?;
        if scene_item_index < 0 || scene_item_index as usize >= scene.items.len() {
            return Err(SetSceneItemIndexError::InvalidSceneItemIndex);
        }
        let Some(current_index) = scene
            .items
            .iter()
            .position(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(SetSceneItemIndexError::SceneItemNotFound);
        };
        let target_index = scene_item_index as usize;
        if current_index != target_index {
            let moved = scene.items.remove(current_index);
            scene.items.insert(target_index, moved);
        }
        Ok(SetSceneItemIndexResult {
            changed: current_index != target_index,
            scene_items: scene
                .items
                .iter()
                .enumerate()
                .map(|(index, item)| ObswsSceneItemIndexEntry {
                    scene_item_id: item.scene_item_id,
                    scene_item_index: index as i64,
                })
                .collect(),
        })
    }

    pub fn set_scene_item_enabled(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
        enabled: bool,
    ) -> Result<SetSceneItemEnabledResult, SceneItemLookupError> {
        let Some(scene) = self.scenes_by_name.get_mut(scene_name) else {
            return Err(SceneItemLookupError::SceneNotFound);
        };
        let Some(scene_item) = scene
            .items
            .iter_mut()
            .find(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(SceneItemLookupError::SceneItemNotFound);
        };
        let changed = scene_item.enabled != enabled;
        scene_item.enabled = enabled;
        Ok(SetSceneItemEnabledResult { changed })
    }

    pub fn get_scene_item_enabled(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<bool, SceneItemLookupError> {
        let Some(scene) = self.scenes_by_name.get(scene_name) else {
            return Err(SceneItemLookupError::SceneNotFound);
        };
        let Some(scene_item) = scene
            .items
            .iter()
            .find(|item| item.scene_item_id == scene_item_id)
        else {
            return Err(SceneItemLookupError::SceneItemNotFound);
        };
        Ok(scene_item.enabled)
    }

    pub fn get_scene_item_locked(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<bool, SceneItemLookupError> {
        let scene_item = self.find_scene_item(scene_name, scene_item_id)?;
        Ok(scene_item.locked)
    }

    pub fn set_scene_item_locked(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
        locked: bool,
    ) -> Result<SetSceneItemLockedResult, SceneItemLookupError> {
        let scene_item = self.find_scene_item_mut(scene_name, scene_item_id)?;
        let changed = scene_item.locked != locked;
        scene_item.locked = locked;
        Ok(SetSceneItemLockedResult { changed })
    }

    pub fn get_scene_item_blend_mode(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<ObswsSceneItemBlendMode, SceneItemLookupError> {
        let scene_item = self.find_scene_item(scene_name, scene_item_id)?;
        Ok(scene_item.blend_mode)
    }

    pub fn set_scene_item_blend_mode(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
        blend_mode: ObswsSceneItemBlendMode,
    ) -> Result<SetSceneItemBlendModeResult, SceneItemLookupError> {
        let scene_item = self.find_scene_item_mut(scene_name, scene_item_id)?;
        let changed = scene_item.blend_mode != blend_mode;
        scene_item.blend_mode = blend_mode;
        Ok(SetSceneItemBlendModeResult { changed })
    }

    pub fn get_scene_item_transform(
        &self,
        scene_name: &str,
        scene_item_id: i64,
    ) -> Result<ObswsSceneItemTransform, SceneItemLookupError> {
        let scene_item = self.find_scene_item(scene_name, scene_item_id)?;
        Ok(scene_item.transform.clone())
    }

    pub fn set_scene_item_transform(
        &mut self,
        scene_name: &str,
        scene_item_id: i64,
        patch: ObswsSceneItemTransformPatch,
    ) -> Result<SetSceneItemTransformResult, SceneItemLookupError> {
        let scene_item = self.find_scene_item_mut(scene_name, scene_item_id)?;
        let mut updated = scene_item.transform.clone();
        let mut changed = false;

        apply_transform_patch_value(&mut changed, &mut updated.position_x, patch.position_x);
        apply_transform_patch_value(&mut changed, &mut updated.position_y, patch.position_y);
        apply_transform_patch_value(&mut changed, &mut updated.rotation, patch.rotation);
        apply_transform_patch_value(&mut changed, &mut updated.scale_x, patch.scale_x);
        apply_transform_patch_value(&mut changed, &mut updated.scale_y, patch.scale_y);
        apply_transform_patch_value(&mut changed, &mut updated.alignment, patch.alignment);
        apply_transform_patch_value(&mut changed, &mut updated.bounds_type, patch.bounds_type);
        apply_transform_patch_value(
            &mut changed,
            &mut updated.bounds_alignment,
            patch.bounds_alignment,
        );
        apply_transform_patch_value(&mut changed, &mut updated.bounds_width, patch.bounds_width);
        apply_transform_patch_value(
            &mut changed,
            &mut updated.bounds_height,
            patch.bounds_height,
        );
        apply_transform_patch_value(&mut changed, &mut updated.crop_top, patch.crop_top);
        apply_transform_patch_value(&mut changed, &mut updated.crop_bottom, patch.crop_bottom);
        apply_transform_patch_value(&mut changed, &mut updated.crop_left, patch.crop_left);
        apply_transform_patch_value(&mut changed, &mut updated.crop_right, patch.crop_right);
        apply_transform_patch_value(
            &mut changed,
            &mut updated.crop_to_bounds,
            patch.crop_to_bounds,
        );

        if changed {
            scene_item.transform = updated.clone();
        }

        Ok(SetSceneItemTransformResult {
            changed,
            scene_item_transform: updated,
        })
    }
}
