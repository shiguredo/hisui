use candle_core::Device;

/// ML 推論用デバイスを自動検出する。
///
/// 試行順序は CUDA → Metal → CPU。GPU 初期化に失敗した場合は warn ログを残して
/// CPU にフォールバックする。各バックエンドのコンテキスト確保は重い手続きなので、
/// 取得した `Device` インスタンスはそのまま返す (再初期化はしない)。
pub fn select_device() -> Device {
    #[cfg(feature = "candle-cuda")]
    match Device::new_cuda(0) {
        Ok(device) => {
            tracing::info!("ML device auto-detected: cuda");
            return device;
        }
        Err(err) => {
            tracing::warn!(
                "requested cuda device unavailable, falling back to next backend: {}",
                err
            );
        }
    }

    #[cfg(feature = "candle-metal")]
    match Device::new_metal(0) {
        Ok(device) => {
            tracing::info!("ML device auto-detected: metal");
            return device;
        }
        Err(err) => {
            tracing::warn!(
                "requested metal device unavailable, falling back to next backend: {}",
                err
            );
        }
    }

    tracing::info!("ML device auto-detected: cpu");
    Device::Cpu
}
