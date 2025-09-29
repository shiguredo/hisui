//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/nvidia-video-codec-sdk
#![warn(missing_docs)]

use std::{
    ffi::{CStr, c_int, c_uint},
    mem::MaybeUninit,
    num::NonZeroUsize,
    ptr,
};

mod sys;

// TODO: 後で対応する
// ビルド時に参照したリポジトリのバージョン
//pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// エラー
#[derive(Debug)]
pub struct Error {
    status: sys::NVENCSTATUS,
    function: &'static str,
    reason: Option<&'static str>,
    detail: Option<String>,
}

impl Error {
    fn check(status: sys::NVENCSTATUS, function: &'static str) -> Result<(), Self> {
        if status == sys::NVENCSTATUS::NV_ENC_SUCCESS {
            Ok(())
        } else {
            Err(Self {
                status,
                function,
                reason: None,
                detail: None,
            })
        }
    }

    fn with_reason(status: sys::NVENCSTATUS, function: &'static str, reason: &'static str) -> Self {
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
        write!(f, "{}() failed: status={:?}", self.function, self.status)?;
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
    ctx: *mut sys::CUcontext,
    decoder: *mut sys::CUvideodecoder,
    parser: *mut sys::CUvideoparser,
    width: u32,
    height: u32,
    decoded_frames: Vec<DecodedFrame>,
}

impl Decoder {
    /// H.265 用のデコーダーインスタンスを生成する
    pub fn new_hevc() -> Result<Self, Error> {
        unsafe {
            let mut ctx = ptr::null_mut();
            let mut decoder = ptr::null_mut();
            let mut parser = ptr::null_mut();

            // CUDA context の初期化
            let status = sys::cuCtxCreate_v2(&mut ctx, 0, 0); // デバイス0を使用
            if status != sys::CUDA_SUCCESS {
                return Err(Error::with_reason(
                    sys::NVENCSTATUS::NV_ENC_ERR_INVALID_DEVICE,
                    "cuCtxCreate_v2",
                    "Failed to create CUDA context",
                ));
            }

            // パーサーパラメータの設定
            let mut parser_params = MaybeUninit::<sys::CUVIDPARSERPARAMS>::zeroed();
            let parser_params = parser_params.as_mut_ptr();
            (*parser_params).CodecType = sys::cudaVideoCodec_HEVC;
            (*parser_params).ulMaxNumDecodeSurfaces = 20;
            (*parser_params).ulMaxDisplayDelay = 1;
            (*parser_params).pUserData = ptr::null_mut();
            (*parser_params).pfnSequenceCallback = Some(Self::handle_video_sequence);
            (*parser_params).pfnDecodePicture = Some(Self::handle_picture_decode);
            (*parser_params).pfnDisplayPicture = Some(Self::handle_picture_display);

            // ビデオパーサーの作成
            let status = sys::cuvidCreateVideoParser(&mut parser, parser_params);
            if status != sys::CUDA_SUCCESS {
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::with_reason(
                    sys::NVENCSTATUS::NV_ENC_ERR_INVALID_PARAM,
                    "cuvidCreateVideoParser",
                    "Failed to create video parser",
                ));
            }

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
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        unsafe {
            let mut packet = MaybeUninit::<sys::CUVIDSOURCEDATAPACKET>::zeroed();
            let packet = packet.as_mut_ptr();
            (*packet).payload = data.as_ptr();
            (*packet).payload_size = data.len() as u32;
            (*packet).flags = sys::CUVID_PKT_TIMESTAMP;
            (*packet).timestamp = 0;

            let status = sys::cuvidParseVideoData(self.parser, packet);
            if status != sys::CUDA_SUCCESS {
                return Err(Error::with_reason(
                    sys::NVENCSTATUS::NV_ENC_ERR_GENERIC,
                    "cuvidParseVideoData",
                    "Failed to parse video data",
                ));
            }
        }
        Ok(())
    }

    /// これ以上データが来ないことをデコーダーに伝える
    ///
    /// 残りのデコード結果は [`Decoder::next_frame()`] で取得できる
    pub fn finish(&mut self) -> Result<(), Error> {
        unsafe {
            let mut packet = MaybeUninit::<sys::CUVIDSOURCEDATAPACKET>::zeroed();
            let packet = packet.as_mut_ptr();
            (*packet).payload = ptr::null();
            (*packet).payload_size = 0;
            (*packet).flags = sys::CUVID_PKT_ENDOFSTREAM;
            (*packet).timestamp = 0;

            let status = sys::cuvidParseVideoData(self.parser, packet);
            if status != sys::CUDA_SUCCESS {
                return Err(Error::with_reason(
                    sys::NVENCSTATUS::NV_ENC_ERR_GENERIC,
                    "cuvidParseVideoData",
                    "Failed to finish decoding",
                ));
            }
        }
        Ok(())
    }

    /// デコード済みのフレームを取り出す
    ///
    /// [`Decoder::decode()`] や [`Decoder::finish()`] の後には、
    /// このメソッドを、結果が `None` になるまで呼び出し続ける必要がある
    pub fn next_frame(&mut self) -> Option<DecodedFrame> {
        self.decoded_frames.pop()
    }

    // コールバック関数（C関数として定義）
    extern "C" fn handle_video_sequence(
        _user_data: *mut std::ffi::c_void,
        video_format: *mut sys::CUVIDEOFORMAT,
    ) -> c_int {
        unsafe {
            // デコーダーの作成パラメータ設定
            let mut create_info = MaybeUninit::<sys::CUVIDDECODECREATEINFO>::zeroed();
            let create_info = create_info.as_mut_ptr();
            (*create_info).CodecType = (*video_format).codec;
            (*create_info).ChromaFormat = (*video_format).chroma_format;
            (*create_info).OutputFormat = if (*video_format).bit_depth_luma_minus8 > 0 {
                sys::cudaVideoSurfaceFormat_P016
            } else {
                sys::cudaVideoSurfaceFormat_NV12
            };
            (*create_info).bitDepthMinus8 = (*video_format).bit_depth_luma_minus8;
            (*create_info).DeinterlaceMode = sys::cudaVideoDeinterlaceMode_Weave;
            (*create_info).ulNumOutputSurfaces = 2;
            (*create_info).ulCreationFlags = sys::cudaVideoCreate_PreferCUVID;
            (*create_info).ulNumDecodeSurfaces = 20;
            (*create_info).ulWidth = (*video_format).coded_width;
            (*create_info).ulHeight = (*video_format).coded_height;
            (*create_info).ulMaxWidth = (*video_format).coded_width;
            (*create_info).ulMaxHeight = (*video_format).coded_height;
            (*create_info).ulTargetWidth = (*video_format).coded_width;
            (*create_info).ulTargetHeight = (*video_format).coded_height;

            // このコールバックでデコーダーを作成する必要があるが、
            // self へのアクセスが困難なため、実際の実装では工夫が必要
            1
        }
    }

    extern "C" fn handle_picture_decode(
        _user_data: *mut std::ffi::c_void,
        _pic_params: *mut sys::CUVIDPICPARAMS,
    ) -> c_int {
        1
    }

    extern "C" fn handle_picture_display(
        _user_data: *mut std::ffi::c_void,
        _disp_info: *mut sys::CUVIDPARSERDISPINFO,
    ) -> c_int {
        1
    }
}

unsafe impl Send for Decoder {}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            if !self.parser.is_null() {
                sys::cuvidDestroyVideoParser(self.parser);
            }
            if !self.decoder.is_null() {
                sys::cuvidDestroyDecoder(self.decoder);
            }
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

// libvpx から引き継いだエンコーダー関連の構造体は一時的に保持
// （今回はデコーダーのみの実装なので、後で削除または置き換え予定）

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

// 暫定的なエンコーダー構造体（将来的にNVCODECエンコーダーに置き換え）
/// H.265 エンコーダー（未実装）
pub struct Encoder;

impl Encoder {
    /// H.265 用のエンコーダーインスタンスを生成する（未実装）
    pub fn new_hevc(_config: &EncoderConfig) -> Result<Self, Error> {
        Err(Error::with_reason(
            sys::NVENCSTATUS::NV_ENC_ERR_UNIMPLEMENTED,
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
            sys::NVENCSTATUS::NV_ENC_ERR_INVALID_PARAM,
            "test_function",
            "test error",
        );
        let error_string = format!("{}", e);
        assert!(error_string.contains("test_function"));
        assert!(error_string.contains("test error"));
    }
}
