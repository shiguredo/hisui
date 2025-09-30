//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/nvidia-video-codec-sdk
#![warn(missing_docs)]

use std::sync::Once;

mod decode;
mod sys;

pub use decode::{DecodedFrame, Decoder};

// ビルド時に参照したリポジトリのバージョン
// Note: sys module doesn't export BUILD_METADATA_VERSION, so this is commented out
// pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// CUDA ドライバーの初期化（プロセスごとに1回だけ実行される）
static CUDA_INIT: Once = Once::new();
static mut CUDA_INIT_RESULT: Option<Result<(), Error>> = None;

/// CUDA ドライバーを初期化する（内部使用）
fn ensure_cuda_initialized() -> Result<(), Error> {
    unsafe {
        CUDA_INIT.call_once(|| {
            let status = sys::cuInit(0);
            CUDA_INIT_RESULT = Some(if status == sys::cudaError_enum_CUDA_SUCCESS {
                Ok(())
            } else {
                Err(Error::with_reason(
                    status,
                    "cuInit",
                    "Failed to initialize CUDA driver",
                ))
            });
        });

        // CUDA_INIT_RESULT は call_once の中で必ず初期化されるため unwrap は安全
        // Use raw pointer instead of reference to avoid static_mut_refs lint
        std::ptr::addr_of!(CUDA_INIT_RESULT)
            .read()
            .as_ref()
            .unwrap()
            .clone()
    }
}

/// エラー
#[derive(Debug, Clone)]
pub struct Error {
    status: u32, // NVENCSTATUS は u32 型
    function: &'static str,
    reason: Option<&'static str>,
    detail: Option<String>,
}

impl Error {
    fn with_reason(status: u32, function: &'static str, reason: &'static str) -> Self {
        Self {
            status,
            function,
            reason: Some(reason),
            detail: None,
        }
    }

    fn reason(&self) -> &str {
        self.reason.unwrap_or("Unknown NVCODEC error")
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}() failed: status={}", self.function, self.status)?;
        write!(f, ", reason={}", self.reason())?;
        if let Some(detail) = &self.detail {
            write!(f, ", detail={detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_hevc_decoder() {
        let _decoder = Decoder::new_hevc().expect("Failed to initialize HEVC decoder");
        println!("HEVC decoder initialized successfully");
    }

    #[test]
    fn test_multiple_decoders() {
        // CUDA初期化が1回だけ実行されることを確認するため、複数のデコーダーを作成
        let _decoder1 = Decoder::new_hevc().expect("Failed to initialize first HEVC decoder");
        let _decoder2 = Decoder::new_hevc().expect("Failed to initialize second HEVC decoder");
        println!("Multiple HEVC decoders initialized successfully");
    }

    #[test]
    fn error_display() {
        let e = Error::with_reason(
            sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM,
            "test_function",
            "test error",
        );
        let error_string = format!("{}", e);
        assert!(error_string.contains("test_function"));
        assert!(error_string.contains("test error"));
    }

    #[test]
    fn test_decode_black_frame() {
        // H.265の黒フレームデータ (Annex B format with start codes)
        let vps = vec![
            64, 1, 12, 1, 255, 255, 1, 96, 0, 0, 3, 0, 144, 0, 0, 3, 0, 0, 3, 0, 90, 149, 152, 9,
        ];
        let sps = vec![
            66, 1, 1, 1, 96, 0, 0, 3, 0, 144, 0, 0, 3, 0, 0, 3, 0, 90, 160, 5, 2, 1, 225, 101, 149,
            154, 73, 50, 188, 5, 160, 32, 0, 0, 3, 0, 32, 0, 0, 3, 3, 33,
        ];
        let pps = vec![68, 1, 193, 114, 180, 98, 64];
        let frame_data = vec![
            40, 1, 175, 29, 16, 90, 181, 140, 90, 213, 247, 1, 91, 255, 242, 78, 254, 199, 0, 31,
            209, 50, 148, 21, 162, 38, 146, 0, 0, 3, 1, 203, 169, 113, 202, 5, 24, 129, 39, 128, 0,
            0, 3, 0, 7, 204, 147, 13, 148, 32, 0, 0, 3, 0, 0, 3, 0, 12, 24, 135, 0, 0, 3, 0, 0, 3,
            0, 0, 3, 0, 28, 240, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 8, 104, 0, 0, 3, 0, 0, 3, 0, 0, 3,
            0, 104, 192, 0, 0, 3, 0, 0, 3, 0, 0, 3, 1, 223, 0, 0, 3, 0, 9, 248,
        ];

        // NALユニットを結合（Annex B形式: start code 0x00000001 を使用）
        let mut h265_data = Vec::new();
        let start_code = [0u8, 0, 0, 1];

        // VPS
        h265_data.extend_from_slice(&start_code);
        h265_data.extend_from_slice(&vps);

        // SPS
        h265_data.extend_from_slice(&start_code);
        h265_data.extend_from_slice(&sps);

        // PPS
        h265_data.extend_from_slice(&start_code);
        h265_data.extend_from_slice(&pps);

        // Frame data
        h265_data.extend_from_slice(&start_code);
        h265_data.extend_from_slice(&frame_data);

        let mut decoder = Decoder::new_hevc().expect("Failed to create HEVC decoder");

        // デコードを実行
        decoder
            .decode(&h265_data)
            .expect("Failed to decode H.265 data");

        // フィニッシュ処理をテスト
        decoder.finish().expect("Failed to finish decoding");

        // デコード済みフレームを取得
        let frame = decoder.next_frame().expect("No decoded frame available");

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y平面とUV平面のデータサイズを確認
        // Note: The actual data size uses pitch (stride), not width, due to GPU alignment
        assert_eq!(frame.y_plane().len(), frame.y_stride() * frame.height());
        assert_eq!(
            frame.uv_plane().len(),
            frame.uv_stride() * frame.height() / 2
        );

        // ストライドが幅以上であることを確認（GPUアラインメントのため）
        assert!(frame.y_stride() >= frame.width());
        assert!(frame.uv_stride() >= frame.width());

        // 黒画面なので、Y成分は16付近、UV成分は128付近の値になることを確認
        let y_data = frame.y_plane();
        let uv_data = frame.uv_plane();

        // Y成分の平均値をチェック（完全な黒は16）
        let y_avg = y_data.iter().map(|&x| x as u32).sum::<u32>() / y_data.len() as u32;
        assert!(
            y_avg >= 10 && y_avg <= 30,
            "Y average should be around 16 for black, got {}",
            y_avg
        );

        // UV成分の平均値をチェック
        // Note: The actual encoded frame may not have perfectly neutral chroma
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            uv_avg >= 70 && uv_avg <= 140,
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded H.265 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);
    }
}
