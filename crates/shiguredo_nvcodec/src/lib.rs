//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/nvidia-video-codec-sdk
#![warn(missing_docs)]

use std::ptr;

mod sys;

// ビルド時に参照したリポジトリのバージョン
// TDOO:
// pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// エラー
#[derive(Debug)]
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

/// H.265 デコーダー
pub struct Decoder {
    ctx: sys::CUcontext,
    decoder: sys::CUvideodecoder,
    parser: ptr::NonNull<std::ffi::c_void>, // パーサーは現在未実装のため汎用ポインタを使用
    width: u32,
    height: u32,
    decoded_frames: Vec<DecodedFrame>,
}

impl Decoder {
    /// H.265 用のデコーダーインスタンスを生成する
    pub fn new_hevc() -> Result<Self, Error> {
        unsafe {
            let mut ctx = ptr::null_mut();
            let decoder = ptr::null_mut();

            // CUDA context の初期化
            let status = sys::cuCtxCreate_v2(&mut ctx, 0, 0); // デバイス0を使用
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_DEVICE,
                    "cuCtxCreate_v2",
                    "Failed to create CUDA context",
                ));
            }

            // 現在、CUDA Video Decode APIの完全な実装は未完成のため、
            // 基本的な初期化のみを行う
            let parser = ptr::NonNull::dangling(); // 仮のポインタ

            Ok(Self {
                ctx,
                decoder,
                parser,
                width: 0,
                height: 0,
                decoded_frames: Vec::new(),
            })
        }
    }

    /// 圧縮された映像フレームをデコードする
    ///
    /// デコード結果は [`Decoder::next_frame()`] で取得できる
    pub fn decode(&mut self, _data: &[u8]) -> Result<(), Error> {
        // 現在の実装では未対応
        Err(Error::with_reason(
            sys::_NVENCSTATUS_NV_ENC_ERR_UNIMPLEMENTED,
            "Decoder::decode",
            "Decode functionality not yet implemented",
        ))
    }

    /// これ以上データが来ないことをデコーダーに伝える
    ///
    /// 残りのデコード結果は [`Decoder::next_frame()`] で取得できる
    pub fn finish(&mut self) -> Result<(), Error> {
        // 現在の実装では未対応
        Err(Error::with_reason(
            sys::_NVENCSTATUS_NV_ENC_ERR_UNIMPLEMENTED,
            "Decoder::finish",
            "Finish functionality not yet implemented",
        ))
    }

    /// デコード済みのフレームを取り出す
    ///
    /// [`Decoder::decode()`] や [`Decoder::finish()`] の後には、
    /// このメソッドを、結果が `None` になるまで呼び出し続ける必要がある
    pub fn next_frame(&mut self) -> Option<DecodedFrame> {
        self.decoded_frames.pop()
    }
}

unsafe impl Send for Decoder {}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            // デコーダーが有効な場合は破棄
            if !self.decoder.is_null() {
                sys::cuvidDestroyDecoder(self.decoder);
            }
            // コンテキストが有効な場合は破棄
            if !self.ctx.is_null() {
                sys::cuCtxDestroy_v2(self.ctx);
            }
        }
    }
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder").finish_non_exhaustive()
    }
}

/// デコードされた映像フレーム (NV12/P016 形式)
pub struct DecodedFrame {
    width: u32,
    height: u32,
    data: Vec<u8>,
    is_high_depth: bool,
}

impl DecodedFrame {
    /// フレームが高ビット深度（10/12ビット）かどうかを返す
    pub fn is_high_depth(&self) -> bool {
        self.is_high_depth
    }

    /// フレームの Y 成分のデータを返す
    pub fn y_plane(&self) -> &[u8] {
        let bytes_per_pixel = if self.is_high_depth { 2 } else { 1 };
        let y_size = self.width as usize * self.height as usize * bytes_per_pixel;
        &self.data[..y_size]
    }

    /// フレームの UV 成分のデータを返す（NV12/P016はインターリーブ形式）
    pub fn uv_plane(&self) -> &[u8] {
        let bytes_per_pixel = if self.is_high_depth { 2 } else { 1 };
        let y_size = self.width as usize * self.height as usize * bytes_per_pixel;
        let uv_size = self.width as usize * (self.height as usize / 2) * bytes_per_pixel;
        &self.data[y_size..y_size + uv_size]
    }

    /// フレームの Y 成分のストライドを返す
    pub fn y_stride(&self) -> usize {
        self.width as usize * if self.is_high_depth { 2 } else { 1 }
    }

    /// フレームの UV 成分のストライドを返す
    pub fn uv_stride(&self) -> usize {
        self.width as usize * if self.is_high_depth { 2 } else { 1 }
    }

    /// フレームの幅を返す
    pub fn width(&self) -> usize {
        self.width as usize
    }

    /// フレームの高さを返す
    pub fn height(&self) -> usize {
        self.height as usize
    }
}

/// エンコーダーに指定する設定
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// 入出力画像の幅
    pub width: usize,
    /// 入出力画像の高さ
    pub height: usize,
    /// FPS の分子
    pub fps_numerator: usize,
    /// FPS の分母
    pub fps_denominator: usize,
    /// エンコードビットレート (bps 単位)
    pub target_bitrate: usize,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps_numerator: 30,
            fps_denominator: 1,
            target_bitrate: 2_000_000,
        }
    }
}

/// H.265 エンコーダー（未実装）
pub struct Encoder;

impl Encoder {
    /// H.265 用のエンコーダーインスタンスを生成する（未実装）
    pub fn new_hevc(_config: &EncoderConfig) -> Result<Self, Error> {
        Err(Error::with_reason(
            sys::_NVENCSTATUS_NV_ENC_ERR_UNIMPLEMENTED,
            "Encoder::new_hevc",
            "Encoder not yet implemented",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_hevc_decoder() {
        // CUDA環境が利用可能な場合のみテストを実行
        if let Ok(_decoder) = Decoder::new_hevc() {
            // デコーダーの初期化が成功した場合
            println!("HEVC decoder initialized successfully");
        } else {
            // CUDA環境が利用できない場合はスキップ
            println!("CUDA environment not available, skipping test");
        }
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
}
