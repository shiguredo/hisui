//! src/ml/device.rs の integration テスト。
//!
//! select_device() がランタイム条件に応じた device を返すことを実環境で確認する。

#![cfg(feature = "candle")]

use hisui::ml::device::select_device;

/// select_device() は panic せずに device を返す。
/// candle-cuda / candle-metal がいずれも無効なビルドでは CPU が選ばれる。
#[test]
fn select_device_does_not_panic() {
    let _device = select_device();
}

/// candle-metal 有効ビルドでは Metal device が返る。
#[cfg(feature = "candle-metal")]
#[test]
fn select_device_returns_metal_when_available() {
    let device = select_device();
    assert!(matches!(device, candle_core::Device::Metal(_)));
}

/// candle-cuda 有効ビルドでは CUDA device が返る。
#[cfg(feature = "candle-cuda")]
#[test]
fn select_device_returns_cuda_when_available() {
    let device = select_device();
    assert!(matches!(device, candle_core::Device::Cuda(_)));
}
