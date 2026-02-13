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

    for request in requests {
        let request_json = nojson::json(|f| f.value(request)).to_string();
        let _ = pipeline_handle.rpc(request_json.as_bytes()).await;
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{run_rpc_request_file, validate_rpc_requests_file};

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

    #[tokio::test]
    async fn run_rpc_request_file_accepts_notification_array() -> crate::Result<()> {
        let mut file =
            tempfile::NamedTempFile::new().map_err(|e| crate::Error::new(e.to_string()))?;
        write!(
            file,
            r#"[{{"jsonrpc":"2.0","method":"listProcessors"}},{{"jsonrpc":"2.0","method":"listTracks"}}]"#
        )
        .map_err(|e| crate::Error::new(e.to_string()))?;

        let pipeline = crate::MediaPipeline::new();
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        run_rpc_request_file(file.path(), &handle).await?;

        drop(handle);
        tokio::time::timeout(std::time::Duration::from_secs(5), pipeline_task)
            .await
            .map_err(|e| crate::Error::new(e.to_string()))?
            .map_err(|e| crate::Error::new(e.to_string()))?;

        Ok(())
    }
}
