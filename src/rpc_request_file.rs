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
        // 全て通知なので結果は無視する
        let _ = pipeline_handle.rpc(request.as_raw_str().as_bytes()).await;
    }

    Ok(())
}

fn validate_rpc_requests_file<'text, 'raw>(
    value: nojson::RawJsonValue<'text, 'raw>,
) -> Result<Vec<nojson::RawJsonValue<'text, 'raw>>, nojson::JsonParseError> {
    let requests: Vec<_> = value.to_array()?.collect();
    for request in requests.iter() {
        let maybe_id = crate::jsonrpc::validate_request(*request)?;
        if let Some(id) = maybe_id {
            return Err(id.invalid("startup RPC request must not contain id"));
        }
    }
    Ok(requests)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    #[test]
    fn validate_startup_rpc_requests_accepts_notification_array() -> crate::Result<()> {
        let parsed = nojson::RawJson::parse(r#"[{"jsonrpc":"2.0","method":"listProcessors"}]"#)?;
        let requests = validate_rpc_requests_file(parsed.value())?;

        assert_eq!(requests.len(), 1);
        Ok(())
    }

    #[test]
    fn validate_startup_rpc_requests_rejects_non_array_root() -> crate::Result<()> {
        let parsed = nojson::RawJson::parse(r#"{}"#)?;
        let result = validate_rpc_requests_file(parsed.value());

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn validate_startup_rpc_requests_rejects_request_with_id() -> crate::Result<()> {
        let parsed =
            nojson::RawJson::parse(r#"[{"jsonrpc":"2.0","method":"listProcessors","id":1}]"#)?;
        let result = validate_rpc_requests_file(parsed.value());

        assert!(result.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn run_rpc_request_file_accepts_notification_array() -> crate::Result<()> {
        let mut file = tempfile::NamedTempFile::new()?;
        write!(
            file,
            r#"[{{"jsonrpc":"2.0","method":"listProcessors"}},{{"jsonrpc":"2.0","method":"listTracks"}}]"#
        )?;

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
