use nojson::JsonValueKind;
use orfail::OrFail;

pub async fn run_rpc_request_file(
    path: &std::path::Path,
    pipeline_handle: &crate::MediaPipelineHandle,
) -> orfail::Result<()> {
    let text = std::fs::read_to_string(path)
        .or_fail_with(|e| format!("failed to read file {}: {e}", path.display()))?;
    let parsed = nojson::RawJson::parse(&text)
        .or_fail_with(|e| format!("failed to parse file {}: {e}", path.display()))?;
    let requests = validate_rpc_requests_file(parsed.value())?;

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

fn validate_rpc_requests_file<'text, 'raw>(
    value: nojson::RawJsonValue<'text, 'raw>,
) -> orfail::Result<Vec<nojson::RawJsonValue<'text, 'raw>>> {
    if value.kind() != JsonValueKind::Array {
        return Err(orfail::Failure::new(
            "startup RPC file must be an array of notification requests",
        ));
    }

    let requests: Vec<_> = value
        .to_array()
        .or_fail_with(|e| format!("failed to parse startup RPC file array: {e}"))?
        .collect();

    for (index, request) in requests.iter().copied().enumerate() {
        if request.kind() != JsonValueKind::Object {
            return Err(orfail::Failure::new(format!(
                "startup RPC request at index {index} must be an object"
            )));
        }

        for (name, _) in request.to_object().or_fail_with(|e| {
            format!("failed to parse startup RPC request at index {index}: {e}")
        })? {
            let name = name.to_unquoted_string_str().or_fail_with(|e| {
                format!("failed to parse startup RPC request member at index {index}: {e}")
            })?;
            if name == "id" {
                return Err(orfail::Failure::new(format!(
                    "startup RPC request at index {index} must not contain id"
                )));
            }
        }
    }

    Ok(requests)
}

fn build_rpc_request_json_with_index_id(
    request: nojson::RawJsonValue<'_, '_>,
    index: usize,
) -> orfail::Result<String> {
    if request.kind() != JsonValueKind::Object {
        return Err(orfail::Failure::new(format!(
            "startup RPC request at index {index} must be an object"
        )));
    }

    let members: Vec<_> = request
        .to_object()
        .or_fail_with(|e| format!("failed to parse startup RPC request at index {index}: {e}"))?
        .map(|(name, value)| {
            name.to_unquoted_string_str()
                .map(|name| (name.into_owned(), value))
                .or_fail_with(|e| {
                    format!("failed to parse startup RPC request member name at index {index}: {e}")
                })
        })
        .collect::<orfail::Result<Vec<_>>>()?;

    let id = i64::try_from(index)
        .map_err(|_| orfail::Failure::new(format!("startup RPC index out of range: {index}")))?;

    Ok(nojson::json(|f| {
        f.object(|f| {
            for (name, value) in &members {
                f.member(name, value)?;
            }
            f.member("id", id)
        })
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use orfail::OrFail;

    use super::{build_rpc_request_json_with_index_id, validate_rpc_requests_file};

    #[test]
    fn validate_startup_rpc_requests_accepts_notification_array() -> orfail::Result<()> {
        let parsed =
            nojson::RawJson::parse(r#"[{"jsonrpc":"2.0","method":"listProcessors"}]"#).or_fail()?;

        let requests = validate_rpc_requests_file(parsed.value())?;

        assert_eq!(requests.len(), 1);
        Ok(())
    }

    #[test]
    fn validate_startup_rpc_requests_rejects_non_array_root() -> orfail::Result<()> {
        let parsed = nojson::RawJson::parse(r#"{}"#).or_fail()?;
        let result = validate_rpc_requests_file(parsed.value());

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn validate_startup_rpc_requests_rejects_request_with_id() -> orfail::Result<()> {
        let parsed =
            nojson::RawJson::parse(r#"[{"jsonrpc":"2.0","method":"listProcessors","id":1}]"#)
                .or_fail()?;

        let result = validate_rpc_requests_file(parsed.value());

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn build_startup_rpc_request_json_adds_id() -> orfail::Result<()> {
        let parsed =
            nojson::RawJson::parse(r#"{"jsonrpc":"2.0","method":"listProcessors"}"#).or_fail()?;

        let request_json = build_rpc_request_json_with_index_id(parsed.value(), 7)?;
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
