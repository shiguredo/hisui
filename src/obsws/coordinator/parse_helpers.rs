//! obsws ハンドラ向け JSON フィールド解析ヘルパー。

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
    Missing,
    Invalid(String),
}

/// `RequiredFieldError` を obsws の `requestStatus` にマップしてエラー応答 JSON を作る。
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

/// 必須文字列フィールド (空文字も valid 値として透過するフィールド向け)。
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

/// 必須文字列フィールド (空文字を `Missing` とする識別子用)。
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

/// 必須 i64 フィールド。
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

/// 必須 u32 フィールド (i64 経由 + 範囲外は `Invalid`)。
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

/// オプション文字列フィールド。欠落は `Ok(None)`、null は `Err`、空文字は `Ok(Some(""))` として透過する。
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

    /// `text_overlay.rs::tests` にも同名の複製がある (`#[cfg(test)] mod tests` を跨いだ共有手段がないため)。
    /// シグネチャ変更時は両方を同時に更新すること。
    fn parse_owned_json(text: &str) -> nojson::RawJsonOwned {
        nojson::RawJsonOwned::parse(text).expect("テスト JSON はパース可能であるべき")
    }

    /// 欠落フィールドは省略 (= 現状維持) として `Ok(None)` を返す。
    #[test]
    fn parse_optional_string_returns_none_for_missing_field() {
        let json = parse_owned_json(r#"{"other":"value"}"#);
        let result = parse_optional_string(&json, "missing");
        assert_eq!(result, Ok(None));
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
        assert_eq!(result, Ok(Some("".to_owned())));
    }

    #[test]
    fn parse_optional_string_rejects_non_string_type() {
        let json = parse_owned_json(r#"{"foo":123}"#);
        let err = parse_optional_string(&json, "foo").expect_err("数値型は拒否される");
        assert!(
            err.contains("string"),
            "エラー文言で string 期待を伝える: {err}"
        );
    }

    #[test]
    fn parse_optional_i64_distinguishes_missing_and_null() {
        let missing = parse_owned_json(r#"{}"#);
        assert_eq!(parse_optional_i64(&missing, "x"), Ok(None));

        let null = parse_owned_json(r#"{"x":null}"#);
        assert!(parse_optional_i64(&null, "x").is_err(), "null は拒否される");

        let value = parse_owned_json(r#"{"x":-100}"#);
        assert_eq!(parse_optional_i64(&value, "x"), Ok(Some(-100)));
    }

    #[test]
    fn parse_optional_u32_rejects_negative_value() {
        let json = parse_owned_json(r#"{"size":-1}"#);
        let err = parse_optional_u32(&json, "size").expect_err("負数は拒否される");
        assert!(
            err.contains("u32 range"),
            "エラー文言で u32 範囲外を伝える: {err}"
        );
    }

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

    #[test]
    fn parse_required_string_field_passes_empty_through() {
        let json = parse_owned_json(r#"{"text":""}"#);
        assert_eq!(
            parse_required_string_field(&json, "text"),
            Ok(String::new())
        );
    }

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

    #[test]
    fn parse_required_non_empty_string_rejects_empty() {
        assert_missing(parse_required_non_empty_string(
            &parse_owned_json(r#"{"name":""}"#),
            "name",
        ));
    }

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
