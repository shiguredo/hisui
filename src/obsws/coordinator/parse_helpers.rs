//! obsws ハンドラ間で共有する汎用 JSON フィールド解析ヘルパー。
//!
//! 「欠落 / null / 型違反 / 空文字」 の分類と `RequiredFieldError` への落とし込みまでをここで担い、
//! ハンドラ固有の検証 (例: i32 範囲チェック) は呼び出し側に残す。

use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_FIELD,
};
use crate::obsws::response::build_request_response_error;

/// 必須フィールドのパース失敗種別。
///
/// `Missing` (欠落 / null / 空文字) は `MISSING_REQUEST_FIELD`、
/// `Invalid` (型違反 / 範囲外) は `INVALID_REQUEST_FIELD` にマップする。
#[derive(Debug, PartialEq)]
pub(super) enum RequiredFieldError {
    /// フィールド欠落、明示的な null、または空文字 (識別子として不適)。
    Missing,
    /// 型違反 (string 期待で integer 等) や範囲外 (u32 範囲外の負数等)。
    Invalid(String),
}

/// `RequiredFieldError` を obsws の `requestStatus` にマップしてエラー応答 JSON を作る。
///
/// 呼び出し側ハンドラの match 分岐を簡素化するためのヘルパー。 戻り値の `RawJsonOwned` は
/// 必要に応じて `ObswsCoordinator::build_result_from_response` で `CommandResult` に包む。
pub(super) fn build_required_field_error_response(
    request_type: &str,
    request_id: &str,
    field_name: &str,
    error: RequiredFieldError,
) -> nojson::RawJsonOwned {
    match error {
        RequiredFieldError::Missing => build_request_response_error(
            request_type,
            request_id,
            REQUEST_STATUS_MISSING_REQUEST_FIELD,
            &format!("Missing or empty {field_name} field"),
        ),
        RequiredFieldError::Invalid(message) => build_request_response_error(
            request_type,
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            &message,
        ),
    }
}

/// 必須文字列フィールドを取り出す (空文字も `Ok` として渡す版)。
/// 欠落 / null は `Missing`、型違反は `Invalid`。
/// 空文字を valid 値として透過するフィールド向け。
pub(super) fn parse_required_string_field(
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
pub(super) fn parse_required_non_empty_string(
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
pub(super) fn parse_required_i64_field(
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
pub(super) fn parse_required_u32_field(
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
pub(super) fn parse_optional_string(
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
pub(super) fn parse_optional_i64(
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

/// オプションフィールドを u32 として取り出す。null / 型不一致は `Err`、欠落は `Ok(None)`。
pub(super) fn parse_optional_u32(
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

    /// `parse_required_i64_field`: 文字列を渡されたら Invalid を返す。
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
