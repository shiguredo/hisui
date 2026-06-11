//! メトリクス (`Stats` レジストリ) の外部出力を担うユーティリティ。
//!
//! `Stats` (`src/stats.rs`) はメトリクスの収集・保持だけを担い、
//! 出力形式 (JSON Lines の `type` 規約等) は本モジュール側に集約する。

use std::io::Write as _;

use crate::stats::Stats;

/// Stats レジストリの全メトリクスを `{"type":"metrics", "metrics": ...}` の 1 行 JSON で stdout に出力する。
///
/// プロセス終了直前の best-effort 出力。書き込み失敗時は警告ログのみで return し、終了処理は妨げない
/// (プロセスが直後に exit するため、失敗を呼び出し側に伝える経路がない)。
/// 起動時の `obsws::server::emit_startup_info_to_stdout` とは方針が逆 (あちらは失敗を `Err` で
/// 呼び出し側に伝える)。
pub fn emit_exit_metrics_to_stdout(stats: &Stats) {
    let families = match stats.to_prometheus_json_families() {
        Ok(families) => families,
        Err(e) => {
            tracing::warn!("failed to collect exit metrics: {}", e.display());
            return;
        }
    };
    let line = nojson::object(|f| {
        f.member("type", "metrics")?;
        f.member("metrics", &families)?;
        Ok(())
    });
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    // BrokenPipe は呼び出し側都合 (パイプ閉じ) なので警告せず黙殺する
    if let Err(e) = writeln!(out, "{line}")
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        tracing::warn!("failed to write exit metrics to stdout: {e}");
    }
}
