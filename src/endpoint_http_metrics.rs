use shiguredo_http11::{Request, Response};

pub async fn handle_request(
    request: &Request,
    pipeline_handle: &crate::MediaPipelineHandle,
) -> Response {
    if request.method != "GET" {
        let mut response = Response::new(405, "Method Not Allowed");
        response.add_header("Allow", "GET");
        return response;
    }

    match pipeline_handle.stats().to_prometheus_text() {
        Ok(text) => {
            let mut response = Response::new(200, "OK");
            response.add_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8");
            response.body = text.into_bytes();
            response
        }
        Err(e) => {
            let mut response = Response::new(500, "Internal Server Error");
            response.add_header("Content-Type", "text/plain; charset=utf-8");
            response.body = format!("failed to render Prometheus metrics: {e}").into_bytes();
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metrics_endpoint_rejects_non_get() {
        let pipeline = crate::MediaPipeline::new().expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let request = Request::new("POST", "/metrics");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code, 405);
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "Allow" && value == "GET")
        );
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_text() {
        let pipeline = crate::MediaPipeline::new().expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let mut stats = handle.stats();
        stats.counter("requests_total").inc();
        let request = Request::new("GET", "/metrics");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code, 200);
        assert!(response.headers.iter().any(|(name, value)| {
            name == "Content-Type" && value == "text/plain; version=0.0.4; charset=utf-8"
        }));
        let body = String::from_utf8(response.body).expect("body must be valid UTF-8");
        assert!(body.contains("# TYPE hisui_requests_total counter"));
        assert!(body.contains("hisui_requests_total 1"));
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_error_for_invalid_metric_name() {
        let pipeline = crate::MediaPipeline::new().expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let mut stats = handle.stats();
        stats.counter("bad-metric-name").inc();
        let request = Request::new("GET", "/metrics");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code, 500);
        let body = String::from_utf8(response.body).expect("body must be valid UTF-8");
        assert!(body.contains("failed to render Prometheus metrics"));
    }
}
