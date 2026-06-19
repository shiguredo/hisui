use tokio::sync::{mpsc, oneshot};

use crate::ProcessorId;
use crate::mixer::text_overlay::{
    TEXT_OVERLAY_PROCESSOR_ID, TextOverlayError, TextOverlayPatch, TextOverlayRpcMessage,
    TextOverlaySpec, TextOverlaySpecInput, TextOverlayState,
};
use crate::obsws::coordinator::{CommandResult, ObswsCoordinator};
use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_DATA,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
    REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED, REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
    REQUEST_STATUS_RESOURCE_NOT_FOUND,
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

        // 必須フィールド
        let Some(name) = parse_required_string(request_data, "textOverlayName") else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing or empty textOverlayName field",
            );
        };
        let Some(text) = parse_required_string(request_data, "text") else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing text field",
            );
        };
        let Some(x) = parse_required_i64(request_data, "x") else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing or invalid x field",
            );
        };
        let Some(y) = parse_required_i64(request_data, "y") else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing or invalid y field",
            );
        };
        let Some(font_size) = parse_required_u32(request_data, "fontSize") else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing or invalid fontSize field",
            );
        };

        // オプションフィールド
        let font_color_argb = match parse_optional_string(request_data, "fontColor") {
            Some(s) => match parse_argb_color(&s) {
                Ok(v) => v,
                Err(e) => {
                    return self.build_error_result(
                        request_type,
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        &e,
                    );
                }
            },
            None => DEFAULT_FONT_COLOR_ARGB,
        };
        let font_name =
            parse_optional_string(request_data, "fontName").unwrap_or(default_font_name);
        let z = parse_optional_i64(request_data, "z");

        let input = TextOverlaySpecInput {
            text,
            x,
            y,
            font_size,
            font_color_argb,
            font_name,
            z,
        };

        // Processor へ Add リクエストを送る
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
            .send(TextOverlayRpcMessage::Add {
                name,
                input,
                reply_tx,
            })
            .is_err()
        {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "text overlay processor is not running",
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
                "text overlay processor dropped reply channel",
            ),
        }
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

        let Some(name) = parse_required_string(request_data, "textOverlayName") else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing or empty textOverlayName field",
            );
        };

        let font_color_argb = match parse_optional_string(request_data, "fontColor") {
            Some(s) => match parse_argb_color(&s) {
                Ok(v) => Some(v),
                Err(e) => {
                    return self.build_error_result(
                        request_type,
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        &e,
                    );
                }
            },
            None => None,
        };

        let patch = TextOverlayPatch {
            text: parse_optional_string(request_data, "text"),
            x: parse_optional_i64(request_data, "x"),
            y: parse_optional_i64(request_data, "y"),
            font_size: parse_optional_u32(request_data, "fontSize"),
            font_color_argb,
            font_name: parse_optional_string(request_data, "fontName"),
            z: parse_optional_i64(request_data, "z"),
        };

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
            .send(TextOverlayRpcMessage::Update {
                name,
                patch,
                reply_tx,
            })
            .is_err()
        {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "text overlay processor is not running",
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
                "text overlay processor dropped reply channel",
            ),
        }
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

        let Some(name) = parse_required_string(request_data, "textOverlayName") else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing or empty textOverlayName field",
            );
        };

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
            .send(TextOverlayRpcMessage::Remove { name, reply_tx })
            .is_err()
        {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "text overlay processor is not running",
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
                "text overlay processor dropped reply channel",
            ),
        }
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
            .send(TextOverlayRpcMessage::List { reply_tx })
            .is_err()
        {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "text overlay processor is not running",
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
                "text overlay processor dropped reply channel",
            ),
        }
    }

    /// `TextOverlayProcessor` の RPC 送信側を取得する。
    async fn text_overlay_sender(
        &self,
    ) -> Result<mpsc::UnboundedSender<TextOverlayRpcMessage>, String> {
        let handle = self
            .pipeline_handle
            .as_ref()
            .ok_or_else(|| "media pipeline handle is not available".to_owned())?;
        handle
            .get_rpc_sender::<mpsc::UnboundedSender<TextOverlayRpcMessage>>(&ProcessorId::new(
                TEXT_OVERLAY_PROCESSOR_ID,
            ))
            .await
            .map_err(|e| format!("failed to get text overlay rpc sender: {e}"))
    }

    fn build_text_overlay_error_result(
        &self,
        request_type: &str,
        request_id: &str,
        error: TextOverlayError,
    ) -> CommandResult {
        let (code, message) = map_text_overlay_error(&error);
        self.build_error_result(request_type, request_id, code, &message)
    }
}

fn map_text_overlay_error(error: &TextOverlayError) -> (i64, String) {
    match error {
        TextOverlayError::AlreadyExists => (
            REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
            "text overlay already exists".to_owned(),
        ),
        TextOverlayError::NotFound => (
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "text overlay not found".to_owned(),
        ),
        TextOverlayError::InvalidFontName(s) => (
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            format!("invalid fontName: {s}"),
        ),
        TextOverlayError::FontResolveFailed(s) => (
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            format!("fontName resolve failed: {s}"),
        ),
        TextOverlayError::InvalidColor(s) => (
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            format!("invalid fontColor: {s}"),
        ),
        TextOverlayError::InvalidFontSize(s) => (
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            format!("invalid fontSize: {s}"),
        ),
        TextOverlayError::InvalidText(s) => (
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            format!("invalid text: {s}"),
        ),
        TextOverlayError::RenderFailed(s) => (
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            format!("render failed: {s}"),
        ),
        TextOverlayError::LimitExceeded => (
            REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
            "text overlay limit exceeded".to_owned(),
        ),
    }
}

fn text_overlay_state_to_json(state: &TextOverlayState) -> nojson::RawJsonOwned {
    let TextOverlayState { name, spec } = state;
    let spec = spec.clone();
    let name = name.clone();
    nojson::RawJsonOwned::object(move |f| {
        f.member("textOverlayName", &name)?;
        f.member("text", &spec.text)?;
        f.member("x", spec.x)?;
        f.member("y", spec.y)?;
        f.member("fontSize", spec.font_size)?;
        f.member("fontColor", argb_to_hex_string(spec.font_color_argb))?;
        f.member("fontName", &spec.font_name)?;
        f.member("z", spec.z)
    })
}

#[allow(dead_code)]
fn spec_to_json(spec: &TextOverlaySpec) -> nojson::RawJsonOwned {
    let spec = spec.clone();
    nojson::RawJsonOwned::object(move |f| {
        f.member("text", &spec.text)?;
        f.member("x", spec.x)?;
        f.member("y", spec.y)?;
        f.member("fontSize", spec.font_size)?;
        f.member("fontColor", argb_to_hex_string(spec.font_color_argb))?;
        f.member("fontName", &spec.font_name)?;
        f.member("z", spec.z)
    })
}

fn argb_to_hex_string(argb: u32) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        (argb >> 16) & 0xFF,
        (argb >> 8) & 0xFF,
        argb & 0xFF,
        (argb >> 24) & 0xFF,
    )
}

fn parse_argb_color(s: &str) -> Result<u32, String> {
    let stripped = s
        .strip_prefix('#')
        .ok_or_else(|| format!("fontColor must start with '#': {s:?}"))?;
    if stripped.len() != 6 && stripped.len() != 8 {
        return Err(format!("fontColor must be #RRGGBB or #RRGGBBAA: {s:?}"));
    }
    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("fontColor must be hex: {s:?}"));
    }
    let rgb = u32::from_str_radix(&stripped[..6], 16).map_err(|_| format!("invalid hex: {s:?}"))?;
    let a = if stripped.len() == 8 {
        u32::from_str_radix(&stripped[6..8], 16).map_err(|_| format!("invalid hex: {s:?}"))?
    } else {
        0xFF
    };
    Ok((a << 24) | rgb)
}

fn parse_required_string(request_data: &nojson::RawJsonOwned, field: &str) -> Option<String> {
    let value: Option<String> = request_data
        .value()
        .to_member(field)
        .ok()?
        .try_into()
        .ok()?;
    let value = value?;
    if value.is_empty() {
        return None;
    }
    Some(value)
}

fn parse_required_i64(request_data: &nojson::RawJsonOwned, field: &str) -> Option<i64> {
    let v = request_data
        .value()
        .to_member(field)
        .ok()?
        .required()
        .ok()?;
    v.try_into().ok()
}

fn parse_required_u32(request_data: &nojson::RawJsonOwned, field: &str) -> Option<u32> {
    let v = request_data
        .value()
        .to_member(field)
        .ok()?
        .required()
        .ok()?;
    v.try_into().ok()
}

fn parse_optional_string(request_data: &nojson::RawJsonOwned, field: &str) -> Option<String> {
    let v = request_data.value().to_member(field).ok()?;
    let v = v.optional()?;
    let value: String = v.try_into().ok()?;
    if value.is_empty() {
        return None;
    }
    Some(value)
}

fn parse_optional_i64(request_data: &nojson::RawJsonOwned, field: &str) -> Option<i64> {
    let v = request_data.value().to_member(field).ok()?;
    let v = v.optional()?;
    v.try_into().ok()
}

fn parse_optional_u32(request_data: &nojson::RawJsonOwned, field: &str) -> Option<u32> {
    let v = request_data.value().to_member(field).ok()?;
    let v = v.optional()?;
    v.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `#RRGGBB` 形式は A=0xFF とみなされる。
    #[test]
    fn parse_argb_color_handles_rgb_only() {
        let v = parse_argb_color("#FF0000").expect("#FF0000 は妥当");
        assert_eq!(v, 0xFFFF0000, "A=0xFF, R=0xFF, G=0x00, B=0x00");
    }

    /// `#RRGGBBAA` 形式が正しく u32 (0xAARRGGBB) に変換される。
    #[test]
    fn parse_argb_color_handles_rgba() {
        let v = parse_argb_color("#FF000080").expect("#FF000080 は妥当");
        assert_eq!(v, 0x80FF0000, "A=0x80, R=0xFF, G=0x00, B=0x00");
    }

    /// 不正な文字種は拒否される。
    #[test]
    fn parse_argb_color_rejects_non_hex() {
        parse_argb_color("#GGGGGG").expect_err("hex 以外は拒否");
    }

    /// `#` プレフィックスがないと拒否。
    #[test]
    fn parse_argb_color_rejects_missing_hash() {
        parse_argb_color("FF0000").expect_err("# 不在は拒否");
    }

    /// 桁数が 6 / 8 以外は拒否。
    #[test]
    fn parse_argb_color_rejects_wrong_length() {
        parse_argb_color("#FFF").expect_err("3 桁は拒否");
        parse_argb_color("#FFFFFFFFF").expect_err("9 桁は拒否");
    }

    /// `argb_to_hex_string` は parse の逆。`#RRGGBBAA` 形式で返す。
    #[test]
    fn argb_to_hex_string_roundtrip() {
        let argb = 0x80FF0000u32;
        let s = argb_to_hex_string(argb);
        assert_eq!(
            s, "#FF000080",
            "A=0x80, R=0xFF, G=0x00, B=0x00 を #RRGGBBAA で出力"
        );
        assert_eq!(
            parse_argb_color(&s).unwrap(),
            argb,
            "parse 後に元の u32 と一致"
        );
    }
}
