// JSON-RPC 2.0 の仕様で定義されているエラーコードの一部
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

pub fn ok_response<I, T>(id: I, result: T) -> nojson::RawJsonOwned
where
    I: nojson::DisplayJson,
    T: nojson::DisplayJson,
{
    let json = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("id", &id)?;
        f.member("result", &result)
    });
    nojson::RawJsonOwned::parse(json.to_string()).expect("infallible")
}

pub fn error_response<I, M>(id: I, code: i32, message: M) -> nojson::RawJsonOwned
where
    I: nojson::DisplayJson,
    M: std::fmt::Display,
{
    let json = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("id", &id)?;
        f.member(
            "error",
            nojson::object(|f| {
                f.member("code", code)?;
                f.member("message", message.to_string())
            }),
        )
    });
    nojson::RawJsonOwned::parse(json.to_string()).expect("infallible")
}

pub fn parse_request_bytes<'a>(
    bytes: &'a [u8],
) -> Result<nojson::RawJson<'a>, nojson::RawJsonOwned> {
    let text = std::str::from_utf8(bytes).map_err(|e| error_response((), PARSE_ERROR, e))?;
    let json = nojson::RawJson::parse(text).map_err(|e| error_response((), PARSE_ERROR, e))?;

    validate_request(json.value()).map_err(|e| error_response((), INVALID_REQUEST, e))?;

    Ok(json)
}

fn validate_request<'text, 'raw>(
    value: nojson::RawJsonValue<'text, 'raw>,
) -> Result<Option<nojson::RawJsonValue<'text, 'raw>>, nojson::JsonParseError> {
    if value.kind() == nojson::JsonValueKind::Array {
        return Err(value.invalid("batch requests are not supported"));
    }

    let mut has_jsonrpc = false;
    let mut has_method = false;
    let mut id = None;
    for (name, value) in value.to_object()? {
        match name.as_string_str()? {
            "jsonrpc" => {
                if value.as_string_str()? != "2.0" {
                    return Err(value.invalid("jsonrpc version must be '2.0'"));
                }
                has_jsonrpc = true;
            }
            "id" => {
                if !matches!(
                    value.kind(),
                    nojson::JsonValueKind::Integer | nojson::JsonValueKind::String
                ) {
                    return Err(value.invalid("id must be an integer or string"));
                }
                id = Some(value);
            }
            "method" => {
                if value.kind() != nojson::JsonValueKind::String {
                    return Err(value.invalid("method must be a string"));
                }
                has_method = true;
            }
            "params" => {
                if !matches!(
                    value.kind(),
                    nojson::JsonValueKind::Object | nojson::JsonValueKind::Array
                ) {
                    return Err(value.invalid("params must be an object or array"));
                }
            }
            _ => {
                // Ignore unknown members
            }
        }
    }

    if !has_jsonrpc {
        return Err(value.invalid("jsonrpc field is required"));
    }
    if !has_method {
        return Err(value.invalid("method field is required"));
    }

    Ok(id)
}
