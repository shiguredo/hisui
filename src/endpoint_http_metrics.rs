use std::fmt::Write as _;

use shiguredo_http11::uri::{Uri, percent_decode};
use shiguredo_http11::{Request, Response};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetricsResponseFormat {
    PrometheusText,
    PrometheusJson,
}

pub async fn handle_request(
    request: &Request,
    pipeline_handle: &crate::MediaPipelineHandle,
) -> Response {
    if request.method() != "GET" {
        // 固定 status / reason / ヘッダー名・値での Response 構築はバリデーションを通る想定。
        // 失敗した場合は実装バグ。
        let mut response =
            Response::new(405, "Method Not Allowed").expect("infallible: fixed status/reason");
        response
            .add_header("Allow", "GET")
            .expect("infallible: fixed header");
        // ボディなしの応答でも Content-Length: 0 を載せたいので空ボディを明示する。
        response.set_body(Vec::new());
        return response;
    }

    let response_format = match parse_metrics_response_format(request.uri()) {
        Ok(response_format) => response_format,
        Err(e) => {
            let mut response =
                Response::new(400, "Bad Request").expect("infallible: fixed status/reason");
            response
                .add_header("Content-Type", "text/plain; charset=utf-8")
                .expect("infallible: fixed header");
            response.set_body(e.into_bytes());
            return response;
        }
    };

    match response_format {
        MetricsResponseFormat::PrometheusText => match pipeline_handle.stats().to_prometheus_text()
        {
            Ok(mut text) => {
                append_tokio_runtime_metrics(&mut text);
                let mut response =
                    Response::new(200, "OK").expect("infallible: fixed status/reason");
                response
                    .add_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
                    .expect("infallible: fixed header");
                response.set_body(text.into_bytes());
                response
            }
            Err(e) => {
                let mut response = Response::new(500, "Internal Server Error")
                    .expect("infallible: fixed status/reason");
                response
                    .add_header("Content-Type", "text/plain; charset=utf-8")
                    .expect("infallible: fixed header");
                response.set_body(
                    format!("failed to render Prometheus metrics: {}", e.display()).into_bytes(),
                );
                response
            }
        },
        MetricsResponseFormat::PrometheusJson => {
            match render_prometheus_json_metrics(pipeline_handle) {
                Ok(json) => {
                    let mut response =
                        Response::new(200, "OK").expect("infallible: fixed status/reason");
                    response
                        .add_header("Content-Type", "application/json; charset=utf-8")
                        .expect("infallible: fixed header");
                    response.set_body(json.to_string().into_bytes());
                    response
                }
                Err(e) => {
                    let mut response = Response::new(500, "Internal Server Error")
                        .expect("infallible: fixed status/reason");
                    response
                        .add_header("Content-Type", "text/plain; charset=utf-8")
                        .expect("infallible: fixed header");
                    response.set_body(
                        format!("failed to render Prometheus metrics JSON: {}", e.display())
                            .into_bytes(),
                    );
                    response
                }
            }
        }
    }
}

fn parse_metrics_response_format(uri: &str) -> Result<MetricsResponseFormat, String> {
    let parsed_uri = Uri::parse(uri).map_err(|e| format!("invalid request URI: {e}"))?;
    let mut format_value = None::<String>;

    if let Some(query) = parsed_uri.query() {
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (raw_name, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
            let name = percent_decode(raw_name).map_err(|e| format!("invalid request URI: {e}"))?;
            if name != "format" {
                continue;
            }
            let value =
                percent_decode(raw_value).map_err(|e| format!("invalid request URI: {e}"))?;
            format_value = Some(value);
        }
    }

    match format_value.as_deref() {
        None => Ok(MetricsResponseFormat::PrometheusText),
        Some("json") => Ok(MetricsResponseFormat::PrometheusJson),
        Some(value) => Err(format!("unsupported metrics format: {value}")),
    }
}

fn render_prometheus_json_metrics(
    pipeline_handle: &crate::MediaPipelineHandle,
) -> crate::Result<nojson::RawJsonOwned> {
    let mut entries = pipeline_handle.stats().entries()?;
    entries.extend(tokio_runtime_metric_entries());
    crate::stats::to_prometheus_json_families_from_entries(entries)
}

fn tokio_runtime_metric_entries() -> Vec<crate::stats::StatsEntry> {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return Vec::new();
    };
    let metrics = handle.metrics();

    vec![
        gauge_stats_entry("tokio_num_workers", usize_to_i64(metrics.num_workers())),
        gauge_stats_entry(
            "tokio_num_alive_tasks",
            usize_to_i64(metrics.num_alive_tasks()),
        ),
        gauge_stats_entry(
            "tokio_global_queue_depth",
            usize_to_i64(metrics.global_queue_depth()),
        ),
    ]
}

fn gauge_stats_entry(metric_name: &'static str, value: i64) -> crate::stats::StatsEntry {
    let gauge = crate::stats::StatsGauge::new();
    gauge.set(value);
    crate::stats::StatsEntry {
        metric_name,
        labels: crate::stats::StatsLabels::default(),
        value: crate::stats::StatsValue::Gauge(gauge),
    }
}

fn usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn append_tokio_runtime_metrics(text: &mut String) {
    for entry in tokio_runtime_metric_entries() {
        text.push_str("# TYPE hisui_");
        text.push_str(entry.metric_name);
        text.push_str(" gauge\n");
        let _ = writeln!(
            text,
            "hisui_{} {}",
            entry.metric_name,
            entry
                .value
                .as_gauge()
                .expect("tokio runtime metrics must be gauge")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の固定リテラル method / uri から Request を作る。
    /// Request::new はバリデーションのため Result を返すがリテラル値は無効になりえないので expect で展開する。
    /// Method は TryFrom<&'static str> しか実装していないので引数も 'static で受ける。
    fn make_request(method: &'static str, uri: &'static str) -> Request {
        Request::new(method, uri).expect("infallible: fixed method/uri")
    }

    #[tokio::test]
    async fn metrics_endpoint_rejects_non_get() {
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
            .expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let request = make_request("POST", "/metrics");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code(), 405);
        assert_eq!(response.get_header("Allow"), Some("GET"));
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_text() {
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
            .expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let mut stats = handle.stats();
        stats.counter("requests_total").inc();
        let request = make_request("GET", "/metrics");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code(), 200);
        assert_eq!(
            response.get_header("Content-Type"),
            Some("text/plain; version=0.0.4; charset=utf-8")
        );
        let body = String::from_utf8(
            response
                .body_bytes()
                .expect("response must have body")
                .to_vec(),
        )
        .expect("body must be valid UTF-8");
        assert!(body.contains("# TYPE hisui_requests_total counter"));
        assert!(body.contains("hisui_requests_total 1"));
        assert!(body.contains("# TYPE hisui_tokio_num_workers gauge"));
        assert!(body.contains("# TYPE hisui_tokio_num_alive_tasks gauge"));
        assert!(body.contains("# TYPE hisui_tokio_global_queue_depth gauge"));
    }

    #[tokio::test]
    async fn metrics_endpoint_does_not_include_removed_sample_entry_counters() {
        // Mp4WriterStats を明示的に初期化したうえで /metrics のレスポンスを取得し、
        // 廃止済みの sample_entry カウンタが含まれないことを確認する。
        // Mp4WriterStats::new を明示的に呼ぶことで、将来 Mp4WriterStats::new 内で
        // 対象カウンタを再追加した場合の回帰を検知する。
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
            .expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let mut stats = handle.stats();
        // `_writer_stats` の値そのものは使わないが、`Mp4WriterStats::new` が `stats` に対して
        // 行う counter 登録を実行することが目的。登録された counter 名が `/metrics` のテキスト
        // 出力に反映されるため、廃止された 4 メトリクスがここで誤って登録されると body に出現する。
        let _writer_stats = crate::mp4::writer::Mp4WriterStats::new(&mut stats, 0);
        let request = make_request("GET", "/metrics");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code(), 200);
        let body = String::from_utf8(
            response
                .body_bytes()
                .expect("response must have body")
                .to_vec(),
        )
        .expect("body must be valid UTF-8");
        assert!(
            !body.contains("hisui_total_received_audio_sample_entry_count"),
            "廃止メトリクス hisui_total_received_audio_sample_entry_count が含まれないこと"
        );
        assert!(
            !body.contains("hisui_total_received_video_sample_entry_count"),
            "廃止メトリクス hisui_total_received_video_sample_entry_count が含まれないこと"
        );
        assert!(
            !body.contains("hisui_total_missing_audio_sample_entry_count"),
            "廃止メトリクス hisui_total_missing_audio_sample_entry_count が含まれないこと"
        );
        assert!(
            !body.contains("hisui_total_missing_video_sample_entry_count"),
            "廃止メトリクス hisui_total_missing_video_sample_entry_count が含まれないこと"
        );
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_error_for_invalid_metric_name() {
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
            .expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let mut stats = handle.stats();
        stats.counter("bad-metric-name").inc();
        let request = make_request("GET", "/metrics");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code(), 500);
        let body = String::from_utf8(
            response
                .body_bytes()
                .expect("response must have body")
                .to_vec(),
        )
        .expect("body must be valid UTF-8");
        assert!(body.contains("failed to render Prometheus metrics"));
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_json() {
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
            .expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let mut stats = handle.stats();
        stats.counter("requests_total").inc();
        let request = make_request("GET", "/metrics?format=json");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code(), 200);
        assert_eq!(
            response.get_header("Content-Type"),
            Some("application/json; charset=utf-8")
        );
        let body = String::from_utf8(
            response
                .body_bytes()
                .expect("response must have body")
                .to_vec(),
        )
        .expect("body must be valid UTF-8");
        assert!(body.contains("\"name\":\"hisui_requests_total\""));
        assert!(body.contains("\"type\":\"COUNTER\""));
        assert!(body.contains("\"value\":\"1\""));
        assert!(body.contains("\"name\":\"hisui_tokio_num_workers\""));
        assert!(body.contains("\"name\":\"hisui_tokio_num_alive_tasks\""));
        assert!(body.contains("\"name\":\"hisui_tokio_global_queue_depth\""));
    }

    #[tokio::test]
    async fn metrics_endpoint_rejects_unsupported_format() {
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
            .expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let request = make_request("GET", "/metrics?format=xml");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code(), 400);
        let body = String::from_utf8(
            response
                .body_bytes()
                .expect("response must have body")
                .to_vec(),
        )
        .expect("body must be valid UTF-8");
        assert!(body.contains("unsupported metrics format"));
    }

    #[tokio::test]
    async fn metrics_endpoint_accepts_percent_encoded_json_format() {
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
            .expect("failed to create media pipeline");
        let handle = pipeline.handle();
        let request = make_request("GET", "/metrics?format=%6a%73%6f%6e");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code(), 200);
        assert_eq!(
            response.get_header("Content-Type"),
            Some("application/json; charset=utf-8")
        );
    }

    #[tokio::test]
    async fn metrics_endpoint_rejects_invalid_percent_encoding() {
        let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())
            .expect("failed to create media pipeline");
        let handle = pipeline.handle();
        // Request::new は URI の percent エンコーディング妥当性まで検証するので、
        // 構文不正な URI (`%ZZ` 等) は Request 構築時に弾かれる。
        // handle_request 内の percent_decode 失敗パスを検証したいので、
        // 構文は正しいが UTF-8 として不正な `%C0%A0` (overlong sequence) を使う。
        let request = make_request("GET", "/metrics?format=%C0%A0");

        let response = handle_request(&request, &handle).await;
        assert_eq!(response.status_code(), 400);
        let body = String::from_utf8(
            response
                .body_bytes()
                .expect("response must have body")
                .to_vec(),
        )
        .expect("body must be valid UTF-8");
        assert!(body.contains("invalid request URI"));
    }
}
