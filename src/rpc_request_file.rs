use nojson::JsonValueKind;

pub async fn run_rpc_request_file(
    path: &std::path::Path,
    pipeline_handle: &crate::MediaPipelineHandle,
) -> crate::Result<()> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| crate::Error::new(format!("failed to read file {}: {e}", path.display())))?;
    let parsed = nojson::RawJson::parse(&text)
        .map_err(|e| crate::Error::new(format!("failed to parse file {}: {e}", path.display())))?;
    let requests = validate_rpc_requests_file(parsed.value())?;

    for (index, request) in requests.into_iter().enumerate() {
        let request_json = build_rpc_request_json_with_index_id(request, index)?;
        let response = pipeline_handle
            .rpc(request_json.as_bytes())
            .await
            .ok_or_else(|| {
                crate::Error::new(format!(
                    "startup RPC request at index {index} returned no response"
                ))
            })?;

        let has_error = response
            .value()
            .to_member("error")
            .map_err(|e| {
                crate::Error::new(format!(
                    "failed to parse startup RPC response at index {index}: {e}"
                ))
            })?
            .get()
            .is_some();
        if has_error {
            return Err(crate::Error::new(format!(
                "startup RPC request at index {index} failed: {}",
                response.text()
            )));
        }
    }

    Ok(())
}

fn validate_rpc_requests_file<'text, 'raw>(
    value: nojson::RawJsonValue<'text, 'raw>,
) -> crate::Result<Vec<nojson::RawJsonValue<'text, 'raw>>> {
    if value.kind() != JsonValueKind::Array {
        return Err(crate::Error::new(
            "startup RPC file must be an array of notification requests",
        ));
    }

    let requests: Vec<_> = value
        .to_array()
        .map_err(|e| crate::Error::new(format!("failed to parse startup RPC file array: {e}")))?
        .collect();

    for (index, request) in requests.iter().copied().enumerate() {
        if request.kind() != JsonValueKind::Object {
            return Err(crate::Error::new(format!(
                "startup RPC request at index {index} must be an object"
            )));
        }

        for (name, _) in request.to_object().map_err(|e| {
            crate::Error::new(format!(
                "failed to parse startup RPC request at index {index}: {e}"
            ))
        })? {
            let name = name.to_unquoted_string_str().map_err(|e| {
                crate::Error::new(format!(
                    "failed to parse startup RPC request member at index {index}: {e}"
                ))
            })?;
            if name == "id" {
                return Err(crate::Error::new(format!(
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
) -> crate::Result<String> {
    if request.kind() != JsonValueKind::Object {
        return Err(crate::Error::new(format!(
            "startup RPC request at index {index} must be an object"
        )));
    }

    let members: Vec<_> = request
        .to_object()
        .map_err(|e| {
            crate::Error::new(format!(
                "failed to parse startup RPC request at index {index}: {e}"
            ))
        })?
        .map(|(name, value)| {
            let name = name.to_unquoted_string_str().map_err(|e| {
                crate::Error::new(format!(
                    "failed to parse startup RPC request member name at index {index}: {e}"
                ))
            })?;
            Ok((name.into_owned(), value))
        })
        .collect::<crate::Result<Vec<_>>>()?;

    let id = i64::try_from(index)
        .map_err(|_| crate::Error::new(format!("startup RPC index out of range: {index}")))?;

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
    use super::{build_rpc_request_json_with_index_id, validate_rpc_requests_file};

    #[test]
    fn validate_startup_rpc_requests_accepts_notification_array() -> crate::Result<()> {
        let parsed = nojson::RawJson::parse(r#"[{"jsonrpc":"2.0","method":"listProcessors"}]"#)
            .map_err(|e| crate::Error::new(e.to_string()))?;

        let requests = validate_rpc_requests_file(parsed.value())?;

        assert_eq!(requests.len(), 1);
        Ok(())
    }

    #[test]
    fn validate_startup_rpc_requests_rejects_non_array_root() -> crate::Result<()> {
        let parsed =
            nojson::RawJson::parse(r#"{}"#).map_err(|e| crate::Error::new(e.to_string()))?;
        let result = validate_rpc_requests_file(parsed.value());

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn validate_startup_rpc_requests_rejects_request_with_id() -> crate::Result<()> {
        let parsed =
            nojson::RawJson::parse(r#"[{"jsonrpc":"2.0","method":"listProcessors","id":1}]"#)
                .map_err(|e| crate::Error::new(e.to_string()))?;

        let result = validate_rpc_requests_file(parsed.value());

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn build_startup_rpc_request_json_adds_id() -> crate::Result<()> {
        let parsed = nojson::RawJson::parse(r#"{"jsonrpc":"2.0","method":"listProcessors"}"#)
            .map_err(|e| crate::Error::new(e.to_string()))?;

        let request_json = build_rpc_request_json_with_index_id(parsed.value(), 7)?;
        let parsed =
            nojson::RawJson::parse(&request_json).map_err(|e| crate::Error::new(e.to_string()))?;
        let id = i64::try_from(
            parsed
                .value()
                .to_member("id")
                .map_err(|e| crate::Error::new(e.to_string()))?
                .required()
                .map_err(|e| crate::Error::new(e.to_string()))?,
        )
        .map_err(|e| crate::Error::new(e.to_string()))?;

        assert_eq!(id, 7);
        Ok(())
    }
}
