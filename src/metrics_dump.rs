//! プロセス終了時に Stats レジストリの全メトリクスを JSON Lines で stdout へ出力する。
//!
//! `--emit-exit-metrics` を main.rs で共通フラグとして受け、subcommand 分岐の return 後に
//! main から呼び出すユーティリティ。JSON Lines のエントリ種別 `type` の付与は出力側の責務として
//! 本モジュールに置き、`Stats` モジュール (`src/stats.rs`) には出力規約を持ち込まない。

use std::io::Write as _;

use crate::stats::Stats;

/// Stats レジストリの全メトリクスを `{"type":"metrics", "metrics": ...}` の 1 行 JSON で stdout に出力する。
///
/// 失敗してもプロセス終了は妨げない（警告ログを出して続行する）。
pub fn emit_exit_metrics_to_stdout(stats: &Stats) {
    let families = match stats.to_prometheus_json_families() {
        Ok(families) => families,
        Err(e) => {
            tracing::warn!("failed to collect metrics for exit dump: {}", e.display());
            return;
        }
    };
    // stdout の JSON Lines ストリームのエントリ種別を `type` で示す（メトリクスダンプは "metrics"）
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
        tracing::warn!("failed to write metrics dump to stdout: {e}");
    }
}
