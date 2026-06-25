use candle_core::Device;

/// ML 推論で使用する device の種別を表す。
///
/// バリアントには feature ゲートを付けない（feature 構成によって enum マッチが
/// 変わって扱いが煩雑になるのを避けるため）。代わりに `auto()` 内部で feature の
/// 有無に応じて到達可能性を制御する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MlDevice {
    Cpu,
    Cuda,
    Metal,
}

impl MlDevice {
    /// 有効化されている feature とランタイム条件に基づいて最適な device を選ぶ。
    ///
    /// 試行順序は CUDA → Metal → CPU。`candle-cuda` / `candle-metal` 双方が
    /// 有効化された特殊ビルドでは CUDA が先に試される（実環境では同時 enable
    /// は起こらない）。GPU 初期化に失敗した場合は warn ログを残してから CPU に
    /// フォールバックする。
    pub fn auto() -> Self {
        #[cfg(feature = "candle-cuda")]
        {
            match Device::new_cuda(0) {
                Ok(_) => {
                    let device = MlDevice::Cuda;
                    tracing::info!("ML device auto-detected: {:?}", device);
                    return device;
                }
                Err(err) => {
                    tracing::warn!(
                        "requested cuda device unavailable, falling back to next backend: {}",
                        err
                    );
                }
            }
        }

        #[cfg(feature = "candle-metal")]
        {
            match Device::new_metal(0) {
                Ok(_) => {
                    let device = MlDevice::Metal;
                    tracing::info!("ML device auto-detected: {:?}", device);
                    return device;
                }
                Err(err) => {
                    tracing::warn!(
                        "requested metal device unavailable, falling back to next backend: {}",
                        err
                    );
                }
            }
        }

        let device = MlDevice::Cpu;
        tracing::info!("ML device auto-detected: {:?}", device);
        device
    }

    /// `candle_core::Device` に変換する。
    ///
    /// `MlDevice::Cuda` / `MlDevice::Metal` バリアントは、対応する feature が
    /// 有効でないビルドでは到達不能（`auto()` が `Cpu` にフォールバックする）
    /// だが、利用者が enum を直接構築した場合の安全策として、未サポートの
    /// バックエンドが指定されたら `Err` を返す（panic はしない）。
    pub fn to_candle_device(self) -> candle_core::Result<Device> {
        match self {
            MlDevice::Cpu => Ok(Device::Cpu),
            #[cfg(feature = "candle-cuda")]
            MlDevice::Cuda => Device::new_cuda(0),
            #[cfg(not(feature = "candle-cuda"))]
            MlDevice::Cuda => Err(candle_core::Error::Msg(
                "candle-cuda feature is not enabled".to_string(),
            )),
            #[cfg(feature = "candle-metal")]
            MlDevice::Metal => Device::new_metal(0),
            #[cfg(not(feature = "candle-metal"))]
            MlDevice::Metal => Err(candle_core::Error::Msg(
                "candle-metal feature is not enabled".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CPU device は常に成功する。
    #[test]
    fn cpu_to_candle_device_succeeds() {
        assert!(MlDevice::Cpu.to_candle_device().is_ok());
    }

    /// auto() の戻り値で to_candle_device() を呼び出しても成功する。
    /// 既定の CI 環境では candle のみが有効で candle-cuda / candle-metal は
    /// 有効化されないので auto() は MlDevice::Cpu を返す前提だが、
    /// 仮にいずれかが有効化されてもフォールバックを経て成功するはず。
    #[test]
    fn auto_returns_usable_device() {
        let device = MlDevice::auto();
        assert!(device.to_candle_device().is_ok());
    }

    /// Metal バックエンドが有効化されたビルドでは Metal device を直接利用できる。
    /// test-apple-toolbox ジョブで実行される。
    #[cfg(feature = "candle-metal")]
    #[test]
    fn metal_device_works() {
        assert!(MlDevice::Metal.to_candle_device().is_ok());
    }

    /// CUDA バックエンドが有効化されたビルドでは CUDA device を直接利用できる。
    /// test-nvidia-video-codec ジョブで実行される。
    #[cfg(feature = "candle-cuda")]
    #[test]
    fn cuda_device_works() {
        assert!(MlDevice::Cuda.to_candle_device().is_ok());
    }
}
