use tokio::sync::{mpsc, oneshot};

use crate::ProcessorId;
use crate::color::Color;
use crate::mixer::video::VideoRealtimeMixerRpcMessage;
use crate::mixer::video::text_overlay::{
    TextOverlayCommand, TextOverlayError, TextOverlayPatch, TextOverlaySpecInput, TextOverlayState,
};
use crate::obsws::coordinator::{CommandResult, ObswsCoordinator};
use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_DATA,
    REQUEST_STATUS_REQUEST_PROCESSING_FAILED, REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};

use super::parse_helpers::{
    parse_optional_i64, parse_optional_string, parse_optional_u32, parse_required_i64_field,
    parse_required_non_empty_string, parse_required_string_field, parse_required_u32_field,
};

/// `fontColor` 省略時のデフォルト値 (不透明白)。
const DEFAULT_FONT_COLOR_ARGB: u32 = 0xFFFFFFFF;

impl ObswsCoordinator {
    pub(crate) async fn handle_create_text_overlay(
        &mut self,
        request_type: &str,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        // Create だけはパース段で default_font_name が必要なので、 ここで機能無効を弾く
        // (Update / Remove / List はパース段で config を参照しないので、
        //  機能無効判定はヘルパ内で 1 回だけ行う)。
        let default_font_name = match self.state.text_overlay_config() {
            Some(c) => c.default_font_name.clone(),
            None => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
                    "text overlay feature is disabled (--font-search-root / --default-font not specified)",
                );
            }
        };

        let Some(request_data) = request_data else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };

        let name = match parse_required_non_empty_string(request_data, "textOverlayName") {
            Ok(s) => s,
            Err(e) => {
                return self.build_required_field_error_result(
                    request_type,
                    request_id,
                    "textOverlayName",
                    e,
                );
            }
        };
        // text は空文字も valid 値として扱う (バイト数 / 行数の上限はミキサー側で検証する)。
        let text = match parse_required_string_field(request_data, "text") {
            Ok(s) => s,
            Err(e) => {
                return self.build_required_field_error_result(request_type, request_id, "text", e);
            }
        };
        let x = match parse_required_i64_field(request_data, "x") {
            Ok(v) => v,
            Err(e) => {
                return self.build_required_field_error_result(request_type, request_id, "x", e);
            }
        };
        let y = match parse_required_i64_field(request_data, "y") {
            Ok(v) => v,
            Err(e) => {
                return self.build_required_field_error_result(request_type, request_id, "y", e);
            }
        };
        let font_size = match parse_required_u32_field(request_data, "fontSize") {
            Ok(v) => v,
            Err(e) => {
                return self.build_required_field_error_result(
                    request_type,
                    request_id,
                    "fontSize",
                    e,
                );
            }
        };

        let font_color_argb = match parse_optional_string(request_data, "fontColor") {
            Ok(Some(s)) => match Color::from_hex(&s) {
                Some(c) => c.to_argb_u32(),
                None => {
                    return self.build_text_overlay_error_result(
                        request_type,
                        request_id,
                        TextOverlayError::InvalidColor(format!(
                            "fontColor must be #RRGGBB or #RRGGBBAA: {s:?}"
                        )),
                    );
                }
            },
            Ok(None) => DEFAULT_FONT_COLOR_ARGB,
            Err(e) => {
                return self.build_invalid_field_error_result(request_type, request_id, &e);
            }
        };
        let font_name = match parse_optional_string(request_data, "fontName") {
            Ok(Some(s)) => s,
            Ok(None) => default_font_name,
            Err(e) => {
                return self.build_invalid_field_error_result(request_type, request_id, &e);
            }
        };
        let z = match parse_optional_z(request_data) {
            Ok(z) => z,
            Err(e) => {
                return self.build_invalid_field_error_result(request_type, request_id, &e);
            }
        };

        let input = TextOverlaySpecInput {
            text,
            x,
            y,
            font_size,
            font_color_argb,
            font_name,
            z,
        };

        self.send_text_overlay_unit_command(request_type, request_id, |reply_tx| {
            TextOverlayCommand::Add {
                name,
                input,
                reply_tx,
            }
        })
        .await
    }

    pub(crate) async fn handle_update_text_overlay(
        &mut self,
        request_type: &str,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        if self.state.text_overlay_config().is_none() {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
                "text overlay feature is disabled",
            );
        }

        let Some(request_data) = request_data else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };

        let name = match parse_required_non_empty_string(request_data, "textOverlayName") {
            Ok(s) => s,
            Err(e) => {
                return self.build_required_field_error_result(
                    request_type,
                    request_id,
                    "textOverlayName",
                    e,
                );
            }
        };

        let font_color_argb = match parse_optional_string(request_data, "fontColor") {
            Ok(Some(s)) => match Color::from_hex(&s) {
                Some(c) => Some(c.to_argb_u32()),
                None => {
                    return self.build_text_overlay_error_result(
                        request_type,
                        request_id,
                        TextOverlayError::InvalidColor(format!(
                            "fontColor must be #RRGGBB or #RRGGBBAA: {s:?}"
                        )),
                    );
                }
            },
            Ok(None) => None,
            Err(e) => {
                return self.build_invalid_field_error_result(request_type, request_id, &e);
            }
        };

        let text = match parse_optional_string(request_data, "text") {
            Ok(v) => v,
            Err(e) => return self.build_invalid_field_error_result(request_type, request_id, &e),
        };
        let x = match parse_optional_i64(request_data, "x") {
            Ok(v) => v,
            Err(e) => return self.build_invalid_field_error_result(request_type, request_id, &e),
        };
        let y = match parse_optional_i64(request_data, "y") {
            Ok(v) => v,
            Err(e) => return self.build_invalid_field_error_result(request_type, request_id, &e),
        };
        let font_size = match parse_optional_u32(request_data, "fontSize") {
            Ok(v) => v,
            Err(e) => return self.build_invalid_field_error_result(request_type, request_id, &e),
        };
        let font_name = match parse_optional_string(request_data, "fontName") {
            Ok(v) => v,
            Err(e) => return self.build_invalid_field_error_result(request_type, request_id, &e),
        };
        let z = match parse_optional_z(request_data) {
            Ok(v) => v,
            Err(e) => return self.build_invalid_field_error_result(request_type, request_id, &e),
        };
        let patch = TextOverlayPatch {
            text,
            x,
            y,
            font_size,
            font_color_argb,
            font_name,
            z,
        };

        self.send_text_overlay_unit_command(request_type, request_id, |reply_tx| {
            TextOverlayCommand::Update {
                name,
                patch,
                reply_tx,
            }
        })
        .await
    }

    pub(crate) async fn handle_remove_text_overlay(
        &mut self,
        request_type: &str,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        if self.state.text_overlay_config().is_none() {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
                "text overlay feature is disabled",
            );
        }

        let Some(request_data) = request_data else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };

        let name = match parse_required_non_empty_string(request_data, "textOverlayName") {
            Ok(s) => s,
            Err(e) => {
                return self.build_required_field_error_result(
                    request_type,
                    request_id,
                    "textOverlayName",
                    e,
                );
            }
        };

        self.send_text_overlay_unit_command(request_type, request_id, |reply_tx| {
            TextOverlayCommand::Remove { name, reply_tx }
        })
        .await
    }

    pub(crate) async fn handle_list_text_overlays(
        &mut self,
        request_type: &str,
        request_id: &str,
        _request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        if self.state.text_overlay_config().is_none() {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
                "text overlay feature is disabled",
            );
        }

        let sender = match self.text_overlay_sender().await {
            Ok(s) => s,
            Err(e) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &e,
                );
            }
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        if sender
            .send(VideoRealtimeMixerRpcMessage::TextOverlay(
                TextOverlayCommand::List { reply_tx },
            ))
            .is_err()
        {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "video mixer rpc channel is closed",
            );
        }
        match reply_rx.await {
            Ok(states) => self.build_result_from_response(
                crate::obsws::response::build_request_response_success(
                    request_type,
                    request_id,
                    move |f| {
                        f.member(
                            "textOverlays",
                            nojson::array(|f| {
                                for s in &states {
                                    f.element(text_overlay_state_to_json(s))?;
                                }
                                Ok(())
                            }),
                        )
                    },
                ),
                Vec::new(),
            ),
            Err(_) => self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "video mixer dropped reply channel",
            ),
        }
    }

    /// Add/Update/Remove の共通骨格 (sender 取得 / send / reply 待ち / エラーマッピング)
    /// を 1 箇所に集約する。 各ハンドラは `build_command` クロージャで
    /// `TextOverlayCommand` のバリアントを組み立てるだけでよい。
    /// 機能無効判定は各ハンドラの先頭で行うため、 ここでは扱わない。
    async fn send_text_overlay_unit_command<F>(
        &self,
        request_type: &str,
        request_id: &str,
        build_command: F,
    ) -> CommandResult
    where
        F: FnOnce(oneshot::Sender<Result<(), TextOverlayError>>) -> TextOverlayCommand,
    {
        let sender = match self.text_overlay_sender().await {
            Ok(s) => s,
            Err(e) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &e,
                );
            }
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        let command = build_command(reply_tx);
        if sender
            .send(VideoRealtimeMixerRpcMessage::TextOverlay(command))
            .is_err()
        {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "video mixer rpc channel is closed",
            );
        }
        match reply_rx.await {
            Ok(Ok(())) => self.build_result_from_response(
                crate::obsws::response::build_request_response_success_no_data(
                    request_type,
                    request_id,
                ),
                Vec::new(),
            ),
            Ok(Err(e)) => self.build_text_overlay_error_result(request_type, request_id, e),
            Err(_) => self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "video mixer dropped reply channel",
            ),
        }
    }

    /// `VideoRealtimeMixer` の RPC 送信側を取得する。
    ///
    /// テキストオーバーレイ機能は `VideoRealtimeMixer` の内部レイヤとして組み込まれて
    /// おり、 `VideoRealtimeMixerRpcMessage::TextOverlay(...)` バリアント経由で
    /// Add / Update / Remove / List を送る。 sender 自体は既存の `program:video_mixer`
    /// のものを共有する (`register_rpc_sender` は同一 processor で 1 sender しか
    /// 許さない)。
    async fn text_overlay_sender(
        &self,
    ) -> Result<mpsc::UnboundedSender<VideoRealtimeMixerRpcMessage>, String> {
        let handle = self
            .pipeline_handle
            .as_ref()
            .ok_or_else(|| "media pipeline handle is not available".to_owned())?;
        handle
            .get_rpc_sender::<mpsc::UnboundedSender<VideoRealtimeMixerRpcMessage>>(
                &ProcessorId::new("program:video_mixer"),
            )
            .await
            .map_err(|e| format!("failed to get video mixer rpc sender: {e}"))
    }

    fn build_text_overlay_error_result(
        &self,
        request_type: &str,
        request_id: &str,
        error: TextOverlayError,
    ) -> CommandResult {
        let code = text_overlay_error_status_code(&error);
        self.build_error_result(request_type, request_id, code, &error.to_string())
    }
}

/// `TextOverlayError` のバリアントから対応する `REQUEST_STATUS_*` コードを返す。
/// クライアント向けの文言は `TextOverlayError::Display` 実装が担当する。
fn text_overlay_error_status_code(error: &TextOverlayError) -> i64 {
    match error {
        TextOverlayError::AlreadyExists => REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
        TextOverlayError::NotFound => REQUEST_STATUS_RESOURCE_NOT_FOUND,
        TextOverlayError::InvalidFontName(_)
        | TextOverlayError::FontResolveFailed(_)
        | TextOverlayError::InvalidColor(_)
        | TextOverlayError::InvalidFontSize(_)
        | TextOverlayError::InvalidText(_) => REQUEST_STATUS_INVALID_REQUEST_FIELD,
        TextOverlayError::LimitExceeded => REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
    }
}

fn text_overlay_state_to_json(state: &TextOverlayState) -> nojson::RawJsonOwned {
    let name = state.name.clone();
    let spec = state.spec.clone();
    nojson::RawJsonOwned::object(move |f| {
        f.member("textOverlayName", &name)?;
        f.member("text", &spec.text)?;
        f.member("x", spec.x)?;
        f.member("y", spec.y)?;
        f.member("fontSize", spec.font_size)?;
        f.member(
            "fontColor",
            Color::from_argb_u32(spec.font_color_argb).to_hex_string(),
        )?;
        f.member("fontName", &spec.font_name)?;
        f.member("z", spec.z)
    })
}

/// `z` フィールドを `Option<i32>` として取り出す。
///
/// `z` は順序値のため i32 範囲で十分な精度。 obsws の他フィールドと同じく
/// JSON 上は integer として受信し、 i32 範囲外はクライアントの誤用とみなして
/// 呼び出し側で `INVALID_REQUEST_FIELD` にマップする。 i32::MAX を含む i32 全域が
/// 有効値で、 予約値は持たない。
fn parse_optional_z(request_data: &nojson::RawJsonOwned) -> Result<Option<i32>, String> {
    let Some(v) = parse_optional_i64(request_data, "z")? else {
        return Ok(None);
    };
    let z = i32::try_from(v).map_err(|_| format!("z must be within i32 range (got {v})"))?;
    Ok(Some(z))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `parse_helpers.rs::tests` にも同名の複製がある (`#[cfg(test)] mod tests` を跨いだ共有手段がないため)。
    /// シグネチャ変更時は両方を同時に更新すること。
    fn parse_owned_json(text: &str) -> nojson::RawJsonOwned {
        nojson::RawJsonOwned::parse(text).expect("テスト JSON はパース可能であるべき")
    }

    /// `parse_optional_z`: i32 範囲外は拒否、欠落は Ok(None)。
    #[test]
    fn parse_optional_z_rejects_out_of_i32_range() {
        let over = parse_owned_json(r#"{"z":2147483648}"#); // i32::MAX + 1
        let err = parse_optional_z(&over).expect_err("i32 範囲超過は拒否される");
        assert!(
            err.contains("i32 range"),
            "エラー文言で i32 範囲外を伝える: {err}"
        );

        let missing = parse_owned_json(r#"{}"#);
        assert_eq!(parse_optional_z(&missing), Ok(None), "欠落は Ok(None)");
    }

    /// `parse_optional_z`: i32::MAX を含む i32 全域が valid な値として受け付けられる。
    #[test]
    fn parse_optional_z_accepts_i32_max() {
        let json = parse_owned_json(r#"{"z":2147483647}"#); // i32::MAX
        assert_eq!(
            parse_optional_z(&json),
            Ok(Some(i32::MAX)),
            "i32::MAX も受け付ける"
        );
    }
}
