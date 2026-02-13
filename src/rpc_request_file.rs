use orfail::OrFail;

pub async fn run_rpc_request_file(
    path: &std::path::Path,
    pipeline_handle: &crate::MediaPipelineHandle,
) -> orfail::Result<()> {
    let value: crate::json::JsonValue = crate::json::parse_file(path)?;
    let requests = validate_rpc_requests_file(value)?;

    for (index, request) in requests.into_iter().enumerate() {
        let request_json = build_rpc_request_json_with_index_id(request, index)?;
        let response = pipeline_handle
            .rpc(request_json.as_bytes())
            .await
            .ok_or_else(|| {
                orfail::Failure::new(format!(
                    "startup RPC request at index {index} returned no response"
                ))
            })?;

        let has_error = response
            .value()
            .to_member("error")
            .or_fail_with(|e| {
                format!("failed to parse startup RPC response at index {index}: {e}")
            })?
            .get()
            .is_some();
        if has_error {
            return Err(orfail::Failure::new(format!(
                "startup RPC request at index {index} failed: {}",
                response.text()
            )));
        }
    }

    Ok(())
}

fn validate_rpc_requests_file(
    value: crate::json::JsonValue,
) -> orfail::Result<Vec<crate::json::JsonValue>> {
    let crate::json::JsonValue::Array(requests) = value else {
        return Err(orfail::Failure::new(
            "startup RPC file must be an array of notification requests",
        ));
    };

    for (index, request) in requests.iter().enumerate() {
        let crate::json::JsonValue::Object(object) = request else {
            return Err(orfail::Failure::new(format!(
                "startup RPC request at index {index} must be an object"
            )));
        };
        if object.contains_key("id") {
            return Err(orfail::Failure::new(format!(
                "startup RPC request at index {index} must not contain id"
            )));
        }
    }

    Ok(requests)
}

fn build_rpc_request_json_with_index_id(
    request: crate::json::JsonValue,
    index: usize,
) -> orfail::Result<String> {
    let crate::json::JsonValue::Object(mut object) = request else {
        return Err(orfail::Failure::new(format!(
            "startup RPC request at index {index} must be an object"
        )));
    };
    let id = i64::try_from(index)
        .map_err(|_| orfail::Failure::new(format!("startup RPC index out of range: {index}")))?;
    object.insert("id".to_owned(), crate::json::JsonValue::Integer(id));

    let request = crate::json::JsonValue::Object(object);
    Ok(nojson::json(|f| f.value(&request)).to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use orfail::OrFail;

    use crate::json::JsonValue;

    use super::{build_rpc_request_json_with_index_id, validate_rpc_requests_file};

    #[test]
    fn validate_startup_rpc_requests_accepts_notification_array() -> orfail::Result<()> {
        let mut request = BTreeMap::new();
        request.insert("jsonrpc".to_owned(), JsonValue::String("2.0".to_owned()));
        request.insert(
            "method".to_owned(),
            JsonValue::String("listProcessors".to_owned()),
        );
        let value = JsonValue::Array(vec![JsonValue::Object(request)]);

        let requests = validate_rpc_requests_file(value)?;

        assert_eq!(requests.len(), 1);
        Ok(())
    }

    #[test]
    fn validate_startup_rpc_requests_rejects_non_array_root() {
        let value = JsonValue::Object(BTreeMap::new());
        let result = validate_rpc_requests_file(value);

        assert!(result.is_err());
    }

    #[test]
    fn validate_startup_rpc_requests_rejects_request_with_id() {
        let mut request = BTreeMap::new();
        request.insert("jsonrpc".to_owned(), JsonValue::String("2.0".to_owned()));
        request.insert(
            "method".to_owned(),
            JsonValue::String("listProcessors".to_owned()),
        );
        request.insert("id".to_owned(), JsonValue::Integer(1));
        let value = JsonValue::Array(vec![JsonValue::Object(request)]);

        let result = validate_rpc_requests_file(value);

        assert!(result.is_err());
    }

    #[test]
    fn build_startup_rpc_request_json_adds_id() -> orfail::Result<()> {
        let mut request = BTreeMap::new();
        request.insert("jsonrpc".to_owned(), JsonValue::String("2.0".to_owned()));
        request.insert(
            "method".to_owned(),
            JsonValue::String("listProcessors".to_owned()),
        );

        let request_json = build_rpc_request_json_with_index_id(JsonValue::Object(request), 7)?;
        let parsed = nojson::RawJson::parse(&request_json).or_fail()?;
        let id = i64::try_from(
            parsed
                .value()
                .to_member("id")
                .or_fail()?
                .required()
                .or_fail()?,
        )
        .or_fail()?;

        assert_eq!(id, 7);
        Ok(())
    }
}
