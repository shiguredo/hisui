use tokio::sync::{mpsc, oneshot};

use crate::ProcessorId;
use crate::mixer::text_overlay::{
    TEXT_OVERLAY_PROCESSOR_ID, TextOverlayError, TextOverlayPatch, TextOverlayRpcMessage,
    TextOverlaySpecInput, TextOverlayState,
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

/// 必須フィールドのパース失敗種別。
///
/// `Missing` (欠落 / null / 空文字) は `MISSING_REQUEST_FIELD`、
/// `Invalid` (型違反 / 範囲外) は `INVALID_REQUEST_FIELD` にマップする。
/// 従来は両者を `Option` で潰していたため型違反まで Missing で返っていた。
#[derive(Debug, PartialEq)]
enum RequiredFieldError {
    /// フィールド欠落、明示的な null、または空文字 (識別子として不適)。
    Missing,
    /// 型違反 (string 期待で integer 等) や範囲外 (u32 範囲外の負数等)。
    Invalid(String),
}

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

        let name = match parse_required_non_empty_string(request_data, "textOverlayName") {
            Ok(s) => s,
            Err(e) => {
                return self.map_required_field_error(
                    request_type,
                    request_id,
                    "textOverlayName",
                    e,
                );
            }
        };
        // text は空文字も valid 値として扱う (issue 仕様: 「最大 4096 バイト、最大 64 行」のみ規定)。
        let text = match parse_required_string_field(request_data, "text") {
            Ok(s) => s,
            Err(e) => return self.map_required_field_error(request_type, request_id, "text", e),
        };
        let x = match parse_required_i64_field(request_data, "x") {
            Ok(v) => v,
            Err(e) => return self.map_required_field_error(request_type, request_id, "x", e),
        };
        let y = match parse_required_i64_field(request_data, "y") {
            Ok(v) => v,
            Err(e) => return self.map_required_field_error(request_type, request_id, "y", e),
        };
        let font_size = match parse_required_u32_field(request_data, "fontSize") {
            Ok(v) => v,
            Err(e) => {
                return self.map_required_field_error(request_type, request_id, "fontSize", e);
            }
        };

        let font_color_argb = match parse_optional_string(request_data, "fontColor") {
            Ok(Some(s)) => match parse_argb_color(&s) {
                Ok(v) => v,
                Err(e) => {
                    return self.build_text_overlay_error_result(request_type, request_id, e);
                }
            },
            Ok(None) => DEFAULT_FONT_COLOR_ARGB,
            Err(e) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &e,
                );
            }
        };
        let font_name = match parse_optional_string(request_data, "fontName") {
            Ok(Some(s)) => s,
            Ok(None) => default_font_name,
            Err(e) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &e,
                );
            }
        };
        let z = match parse_optional_z(request_data) {
            Ok(z) => z,
            Err(e) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &e,
                );
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

        let name = match parse_required_non_empty_string(request_data, "textOverlayName") {
            Ok(s) => s,
            Err(e) => {
                return self.map_required_field_error(
                    request_type,
                    request_id,
                    "textOverlayName",
                    e,
                );
            }
        };

        let font_color_argb = match parse_optional_string(request_data, "fontColor") {
            Ok(Some(s)) => match parse_argb_color(&s) {
                Ok(v) => Some(v),
                Err(e) => {
                    return self.build_text_overlay_error_result(request_type, request_id, e);
                }
            },
            Ok(None) => None,
            Err(e) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &e,
                );
            }
        };

        // null / 型不一致は即 INVALID_REQUEST_FIELD で返すクロージャ。
        let invalid = |e: String| -> CommandResult {
            self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                &e,
            )
        };
        let text = match parse_optional_string(request_data, "text") {
            Ok(v) => v,
            Err(e) => return invalid(e),
        };
        let x = match parse_optional_i64(request_data, "x") {
            Ok(v) => v,
            Err(e) => return invalid(e),
        };
        let y = match parse_optional_i64(request_data, "y") {
            Ok(v) => v,
            Err(e) => return invalid(e),
        };
        let font_size = match parse_optional_u32(request_data, "fontSize") {
            Ok(v) => v,
            Err(e) => return invalid(e),
        };
        let font_name = match parse_optional_string(request_data, "fontName") {
            Ok(v) => v,
            Err(e) => return invalid(e),
        };
        let z = match parse_optional_z(request_data) {
            Ok(v) => v,
            Err(e) => return invalid(e),
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

        let name = match parse_required_non_empty_string(request_data, "textOverlayName") {
            Ok(s) => s,
            Err(e) => {
                return self.map_required_field_error(
                    request_type,
                    request_id,
                    "textOverlayName",
                    e,
                );
            }
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

    /// `RequiredFieldError` を obsws の `requestStatus` にマップして `CommandResult` を作る。
    /// 呼び出し側ハンドラの 5 必須フィールド分の match を簡素化するためのヘルパー。
    fn map_required_field_error(
        &self,
        request_type: &str,
        request_id: &str,
        field_name: &str,
        error: RequiredFieldError,
    ) -> CommandResult {
        match error {
            RequiredFieldError::Missing => self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                &format!("Missing or empty {field_name} field"),
            ),
            RequiredFieldError::Invalid(message) => self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                &message,
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
        let code = text_overlay_error_status_code(&error);
        self.build_error_result(request_type, request_id, code, &error.to_string())
    }
}

/// `TextOverlayError` のバリアントから対応する `REQUEST_STATUS_*` コードを返す。
/// クライアント向けの文言は `TextOverlayError::Display` 実装が担当する
/// (旧 `map_text_overlay_error` で二重に書いていた文言生成をなくしている)。
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

fn parse_argb_color(s: &str) -> Result<u32, TextOverlayError> {
    let stripped = s.strip_prefix('#').ok_or_else(|| {
        TextOverlayError::InvalidColor(format!("fontColor must start with '#': {s:?}"))
    })?;
    if stripped.len() != 6 && stripped.len() != 8 {
        return Err(TextOverlayError::InvalidColor(format!(
            "fontColor must be #RRGGBB or #RRGGBBAA: {s:?}"
        )));
    }
    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(TextOverlayError::InvalidColor(format!(
            "fontColor must be hex: {s:?}"
        )));
    }
    let rgb = u32::from_str_radix(&stripped[..6], 16)
        .map_err(|_| TextOverlayError::InvalidColor(format!("invalid hex: {s:?}")))?;
    let a = if stripped.len() == 8 {
        u32::from_str_radix(&stripped[6..8], 16)
            .map_err(|_| TextOverlayError::InvalidColor(format!("invalid hex: {s:?}")))?
    } else {
        0xFF
    };
    Ok((a << 24) | rgb)
}

/// 必須文字列フィールドを取り出す (空文字も `Ok` として渡す版)。
/// 欠落 / null は `Missing`、型違反は `Invalid`。
/// `text` のように「空文字が valid 値」のフィールド向け。
fn parse_required_string_field(
    request_data: &nojson::RawJsonOwned,
    field: &str,
) -> Result<String, RequiredFieldError> {
    let Ok(member) = request_data.value().to_member(field) else {
        return Err(RequiredFieldError::Missing);
    };
    let Some(v) = member.optional() else {
        return Err(RequiredFieldError::Missing);
    };
    if v.kind().is_null() {
        return Err(RequiredFieldError::Missing);
    }
    v.try_into()
        .map_err(|_| RequiredFieldError::Invalid(format!("field '{field}' must be a string")))
}

/// 必須文字列フィールドを取り出す (空文字も `Missing` とする版)。
/// 識別子フィールド (`textOverlayName` 等) は空文字を valid 値として扱えないため
/// こちらを使う。
fn parse_required_non_empty_string(
    request_data: &nojson::RawJsonOwned,
    field: &str,
) -> Result<String, RequiredFieldError> {
    let s = parse_required_string_field(request_data, field)?;
    if s.is_empty() {
        return Err(RequiredFieldError::Missing);
    }
    Ok(s)
}

/// 必須 i64 フィールドを取り出す。欠落 / null は `Missing`、型違反は `Invalid`。
fn parse_required_i64_field(
    request_data: &nojson::RawJsonOwned,
    field: &str,
) -> Result<i64, RequiredFieldError> {
    let Ok(member) = request_data.value().to_member(field) else {
        return Err(RequiredFieldError::Missing);
    };
    let Some(v) = member.optional() else {
        return Err(RequiredFieldError::Missing);
    };
    if v.kind().is_null() {
        return Err(RequiredFieldError::Missing);
    }
    v.try_into()
        .map_err(|_| RequiredFieldError::Invalid(format!("field '{field}' must be an integer")))
}

/// 必須 u32 フィールドを取り出す。i64 経由で受けてから u32 範囲を確認する。
/// 範囲外 (負数等) は `Invalid` として扱う。
fn parse_required_u32_field(
    request_data: &nojson::RawJsonOwned,
    field: &str,
) -> Result<u32, RequiredFieldError> {
    let v = parse_required_i64_field(request_data, field)?;
    u32::try_from(v).map_err(|_| {
        RequiredFieldError::Invalid(format!(
            "field '{field}' must be within u32 range (got {v})"
        ))
    })
}

/// オプションフィールドを文字列として取り出す。
///
/// - フィールド欠落: `Ok(None)` (省略 = 現状維持)
/// - `null` 値: `Err(...)` (クライアント側の明示的な指定とみなす)
/// - 空文字 `""`: `Ok(Some(""))` (値として受け取り、空文字許否はフィールド固有の検証に委ねる)
/// - 型不一致: `Err(...)`
/// - 正常値: `Ok(Some(value))`
fn parse_optional_string(
    request_data: &nojson::RawJsonOwned,
    field: &str,
) -> Result<Option<String>, String> {
    let Ok(member) = request_data.value().to_member(field) else {
        return Ok(None);
    };
    let Some(v) = member.optional() else {
        return Ok(None);
    };
    if v.kind().is_null() {
        return Err(format!("field '{field}' must not be null"));
    }
    let value: String = v
        .try_into()
        .map_err(|_| format!("field '{field}' must be a string"))?;
    Ok(Some(value))
}

/// オプションフィールドを i64 として取り出す。null / 型不一致は `Err`、欠落は `Ok(None)`。
fn parse_optional_i64(
    request_data: &nojson::RawJsonOwned,
    field: &str,
) -> Result<Option<i64>, String> {
    let Ok(member) = request_data.value().to_member(field) else {
        return Ok(None);
    };
    let Some(v) = member.optional() else {
        return Ok(None);
    };
    if v.kind().is_null() {
        return Err(format!("field '{field}' must not be null"));
    }
    let value: i64 = v
        .try_into()
        .map_err(|_| format!("field '{field}' must be an integer"))?;
    Ok(Some(value))
}

/// `z` フィールドを `Option<i32>` として取り出す。
///
/// `z` は順序値のため i32 範囲で十分な精度だが、obsws の他フィールドと同じく
/// JSON 上は integer として受信する。
/// 許容範囲は `i32::MIN..=i32::MAX - 1` で、`i32::MAX` は
/// `ObswsVideoMixerInputTrack` で text overlay レイヤ用に予約されているため拒否する
/// (一般 input track と同じ z にすると合成順序が壊れる)。
/// 範囲外はクライアントの誤用とみなし、呼び出し側で `INVALID_REQUEST_FIELD` にマップする。
fn parse_optional_z(request_data: &nojson::RawJsonOwned) -> Result<Option<i32>, String> {
    let Some(v) = parse_optional_i64(request_data, "z")? else {
        return Ok(None);
    };
    let z = i32::try_from(v).map_err(|_| format!("z must be within i32 range (got {v})"))?;
    if z == i32::MAX {
        return Err(format!(
            "z = i32::MAX is reserved for the text overlay layer (got {v})"
        ));
    }
    Ok(Some(z))
}

/// オプションフィールドを u32 として取り出す。null / 型不一致は `Err`、欠落は `Ok(None)`。
fn parse_optional_u32(
    request_data: &nojson::RawJsonOwned,
    field: &str,
) -> Result<Option<u32>, String> {
    let v = parse_optional_i64(request_data, field)?;
    let Some(v) = v else { return Ok(None) };
    u32::try_from(v)
        .map(Some)
        .map_err(|_| format!("field '{field}' must be within u32 range (got {v})"))
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

    fn parse_owned_json(text: &str) -> nojson::RawJsonOwned {
        nojson::RawJsonOwned::parse(text).expect("テスト JSON はパース可能であるべき")
    }

    /// 欠落フィールドは省略 (= 現状維持) として `Ok(None)` を返す。
    #[test]
    fn parse_optional_string_returns_none_for_missing_field() {
        let json = parse_owned_json(r#"{"other":"value"}"#);
        let result = parse_optional_string(&json, "missing");
        assert_eq!(result, Ok(None), "欠落は Ok(None)");
    }

    /// 明示的な `null` 値はクライアント側の意図的な指定とみなして拒否する。
    #[test]
    fn parse_optional_string_rejects_null() {
        let json = parse_owned_json(r#"{"foo":null}"#);
        let err = parse_optional_string(&json, "foo").expect_err("null は拒否される");
        assert!(
            err.contains("null"),
            "エラー文言で null 拒否を伝える: {err}"
        );
    }

    /// 空文字は値として呼び出し側に渡す (空文字許否はフィールド固有の検証に委ねる)。
    #[test]
    fn parse_optional_string_passes_through_empty_string() {
        let json = parse_owned_json(r#"{"foo":""}"#);
        let result = parse_optional_string(&json, "foo");
        assert_eq!(
            result,
            Ok(Some("".to_owned())),
            "空文字は Ok(Some(\"\")) として透過する"
        );
    }

    /// 文字列以外の型は拒否する。
    #[test]
    fn parse_optional_string_rejects_non_string_type() {
        let json = parse_owned_json(r#"{"foo":123}"#);
        let err = parse_optional_string(&json, "foo").expect_err("数値型は拒否される");
        assert!(
            err.contains("string"),
            "エラー文言で string 期待を伝える: {err}"
        );
    }

    /// 正常値はそのまま `Ok(Some(value))` で返る。
    #[test]
    fn parse_optional_string_returns_value() {
        let json = parse_owned_json(r#"{"foo":"bar"}"#);
        assert_eq!(
            parse_optional_string(&json, "foo"),
            Ok(Some("bar".to_owned()))
        );
    }

    /// `parse_optional_i64`: 欠落は Ok(None)、null は Err、整数は Ok(Some)。
    #[test]
    fn parse_optional_i64_distinguishes_missing_and_null() {
        let missing = parse_owned_json(r#"{}"#);
        assert_eq!(
            parse_optional_i64(&missing, "x"),
            Ok(None),
            "欠落は Ok(None)"
        );

        let null = parse_owned_json(r#"{"x":null}"#);
        assert!(parse_optional_i64(&null, "x").is_err(), "null は拒否される");

        let value = parse_owned_json(r#"{"x":-100}"#);
        assert_eq!(parse_optional_i64(&value, "x"), Ok(Some(-100)));
    }

    /// `parse_optional_u32`: 負数は範囲外として拒否する。
    #[test]
    fn parse_optional_u32_rejects_negative_value() {
        let json = parse_owned_json(r#"{"size":-1}"#);
        let err = parse_optional_u32(&json, "size").expect_err("負数は拒否される");
        assert!(
            err.contains("u32 range"),
            "エラー文言で u32 範囲外を伝える: {err}"
        );
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

    /// `parse_optional_z`: i32::MAX はテキストオーバーレイレイヤの予約値のため拒否される。
    /// クライアントが指定すると一般 input track と合成順序が衝突するので INVALID で返す。
    #[test]
    fn parse_optional_z_rejects_reserved_i32_max() {
        let json = parse_owned_json(r#"{"z":2147483647}"#); // i32::MAX
        let err = parse_optional_z(&json).expect_err("i32::MAX は予約値のため拒否される");
        assert!(
            err.contains("reserved"),
            "エラー文言で予約値であることを伝える: {err}"
        );

        // 直前値 (i32::MAX - 1) は許容される。
        let ok = parse_owned_json(r#"{"z":2147483646}"#); // i32::MAX - 1
        assert_eq!(
            parse_optional_z(&ok),
            Ok(Some(i32::MAX - 1)),
            "i32::MAX - 1 は許容される"
        );
    }

    // -- 必須フィールドパーサ (Missing と Invalid を区別する版) のテスト --

    fn assert_missing<T: std::fmt::Debug>(result: Result<T, RequiredFieldError>) {
        match result {
            Err(RequiredFieldError::Missing) => {}
            other => panic!("Missing を期待したが {other:?}"),
        }
    }

    fn assert_invalid<T: std::fmt::Debug>(result: Result<T, RequiredFieldError>) {
        match result {
            Err(RequiredFieldError::Invalid(_)) => {}
            other => panic!("Invalid を期待したが {other:?}"),
        }
    }

    /// `parse_required_string_field`: 空文字も valid 値として透過する (text 向け)。
    #[test]
    fn parse_required_string_field_passes_empty_through() {
        let json = parse_owned_json(r#"{"text":""}"#);
        assert_eq!(
            parse_required_string_field(&json, "text"),
            Ok(String::new())
        );
    }

    /// `parse_required_string_field`: 欠落 / null / 型違反の挙動。
    #[test]
    fn parse_required_string_field_classifies_failures() {
        assert_missing(parse_required_string_field(
            &parse_owned_json(r#"{}"#),
            "foo",
        ));
        assert_missing(parse_required_string_field(
            &parse_owned_json(r#"{"foo":null}"#),
            "foo",
        ));
        assert_invalid(parse_required_string_field(
            &parse_owned_json(r#"{"foo":123}"#),
            "foo",
        ));
    }

    /// `parse_required_non_empty_string`: 識別子向けに空文字も Missing として扱う。
    #[test]
    fn parse_required_non_empty_string_rejects_empty() {
        assert_missing(parse_required_non_empty_string(
            &parse_owned_json(r#"{"name":""}"#),
            "name",
        ));
    }

    /// `parse_required_i64_field`: 文字列を渡されたら Invalid を返す
    /// (= 従来は Missing で返って混乱の元だった経路の回帰防止)。
    #[test]
    fn parse_required_i64_field_classifies_type_mismatch_as_invalid() {
        assert_invalid(parse_required_i64_field(
            &parse_owned_json(r#"{"x":"abc"}"#),
            "x",
        ));
        assert_missing(parse_required_i64_field(&parse_owned_json(r#"{}"#), "x"));
        assert_eq!(
            parse_required_i64_field(&parse_owned_json(r#"{"x":-42}"#), "x"),
            Ok(-42)
        );
    }

    /// `parse_required_u32_field`: 負数は範囲外として Invalid。
    #[test]
    fn parse_required_u32_field_rejects_negative() {
        assert_invalid(parse_required_u32_field(
            &parse_owned_json(r#"{"size":-1}"#),
            "size",
        ));
        assert_eq!(
            parse_required_u32_field(&parse_owned_json(r#"{"size":48}"#), "size"),
            Ok(48u32)
        );
    }
}
