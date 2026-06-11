//! メトリクス (`Stats` レジストリ) の外部出力を担うユーティリティ。
//!
//! `Stats` (`src/stats.rs`) はメトリクスの収集・保持だけを担い、
//! 出力形式 (JSON Lines の `type` 規約等) は本モジュール側に集約する。

use std::io::Write as _;

use crate::stats::Stats;

/// Stats レジストリの全メトリクスを `{"type":"metrics", "metrics": ...}` の 1 行 JSON で stdout に出力する。
///
/// 失敗してもプロセス終了は妨げない（警告ログを出して続行する）。
pub fn emit_exit_metrics_to_stdout(stats: &Stats) {
    let families = match stats.to_prometheus_json_families() {
        Ok(families) => families,
        Err(e) => {
            tracing::warn!("failed to collect exit metrics: {}", e.display());
            return;
        }
    };
    // stdout の JSON Lines ストリームのエントリ種別を `type` で示す（終了時メトリクスは "metrics"）
    let line = nojson::object(|f| {
        f.member("type", "metrics")?;
        f.member("metrics", &families)?;
        Ok(())
    });
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    // 出力先のパイプが途中閉じられた場合は警告しない（json.rs::pretty_print と同様）
    if let Err(e) = writeln!(out, "{line}")
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        tracing::warn!("failed to write exit metrics to stdout: {e}");
    }
}
