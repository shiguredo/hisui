use candle_core::Device;

/// ML 推論に使用するデバイス
#[derive(Debug, Clone)]
pub enum MlDevice {
    Cpu,
    Metal(usize),
    Cuda(usize),
}

impl MlDevice {
    /// 利用可能なデバイスを自動検出する
    ///
    /// Metal → CUDA → CPU の順に試行し、最初に利用可能なものを返す
    pub fn auto() -> MlDevice {
        #[cfg(feature = "candle-metal")]
        {
            let metal = MlDevice::Metal(0);
            if metal.to_candle_device().is_ok() {
                tracing::info!("ML device auto-detected: Metal(0)");
                return metal;
            }
        }
        #[cfg(feature = "candle-cuda")]
        {
            let cuda = MlDevice::Cuda(0);
            if cuda.to_candle_device().is_ok() {
                tracing::info!("ML device auto-detected: Cuda(0)");
                return cuda;
            }
        }
        tracing::info!("ML device auto-detected: Cpu");
        MlDevice::Cpu
    }

    /// candle の Device に変換する
    pub fn to_candle_device(&self) -> candle_core::Result<Device> {
        match self {
            MlDevice::Cpu => Ok(Device::Cpu),
            MlDevice::Metal(ordinal) => Device::new_metal(*ordinal),
            MlDevice::Cuda(ordinal) => Device::new_cuda(*ordinal),
        }
    }
}
