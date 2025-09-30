//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/nvidia-video-codec-sdk
#![warn(missing_docs)]

use std::ffi::c_void;
use std::ptr;

mod sys;

// ビルド時に参照したリポジトリのバージョン
// Note: sys module doesn't export BUILD_METADATA_VERSION, so this is commented out
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
    width: u32,
    height: u32,
    decoded_frames: Vec<DecodedFrame>,
}

impl Decoder {
    /// H.265 用のデコーダーインスタンスを生成する
    pub fn new_hevc() -> Result<Self, Error> {
        unsafe {
            // CUDA ドライバーの初期化
            let status = sys::cuInit(0);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuInit",
                    "Failed to initialize CUDA driver",
                ));
            }

            let mut ctx = ptr::null_mut();

            // CUDA context の初期化
            let status = sys::cuCtxCreate_v2(&mut ctx, 0, 0);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuCtxCreate_v2",
                    "Failed to create CUDA context",
                ));
            }

            // Create a context lock for the decoder
            let mut ctx_lock = ptr::null_mut();
            let status = sys::cuvidCtxLockCreate(&mut ctx_lock, ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::with_reason(
                    status,
                    "cuvidCtxLockCreate",
                    "Failed to create context lock",
                ));
            }

            // Video decoder の作成情報を設定
            let mut create_info: sys::CUVIDDECODECREATEINFO = std::mem::zeroed();
            create_info.CodecType = sys::cudaVideoCodec_enum_cudaVideoCodec_HEVC;
            create_info.ChromaFormat = sys::cudaVideoChromaFormat_enum_cudaVideoChromaFormat_420;
            create_info.OutputFormat = sys::cudaVideoSurfaceFormat_enum_cudaVideoSurfaceFormat_NV12;
            create_info.bitDepthMinus8 = 0;
            create_info.DeinterlaceMode =
                sys::cudaVideoDeinterlaceMode_enum_cudaVideoDeinterlaceMode_Weave;
            create_info.ulNumOutputSurfaces = 2;
            create_info.ulCreationFlags =
                sys::cudaVideoCreateFlags_enum_cudaVideoCreate_PreferCUVID as u64;
            create_info.ulNumDecodeSurfaces = 20;
            create_info.ulWidth = 4096;
            create_info.ulHeight = 4096;
            create_info.ulMaxWidth = 4096;
            create_info.ulMaxHeight = 4096;
            create_info.ulTargetWidth = 4096;
            create_info.ulTargetHeight = 4096;
            create_info.vidLock = ctx_lock;

            let mut decoder = ptr::null_mut();

            // Push CUDA context before creating decoder
            let status = sys::cuCtxPushCurrent_v2(ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                sys::cuvidCtxLockDestroy(ctx_lock);
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::with_reason(
                    status,
                    "cuCtxPushCurrent_v2",
                    "Failed to push CUDA context",
                ));
            }

            let status = sys::cuvidCreateDecoder(&mut decoder, &mut create_info);

            // Always pop context
            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            if status != sys::cudaError_enum_CUDA_SUCCESS {
                sys::cuvidCtxLockDestroy(ctx_lock);
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::with_reason(
                    status,
                    "cuvidCreateDecoder",
                    "Failed to create video decoder",
                ));
            }

            Ok(Self {
                ctx,
                decoder,
                width: 0,
                height: 0,
                decoded_frames: Vec::new(),
            })
        }
    }

    /// 圧縮された映像フレームをデコードする
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        if data.is_empty() {
            return Ok(());
        }

        unsafe {
            // Push CUDA context before decoding
            let status = sys::cuCtxPushCurrent_v2(self.ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuCtxPushCurrent_v2",
                    "Failed to push CUDA context",
                ));
            }

            // Decode the H.265 data directly
            let mut pic_params: sys::CUVIDPICPARAMS = std::mem::zeroed();
            pic_params.nBitstreamDataLen = data.len() as u32;
            pic_params.pBitstreamData = data.as_ptr();
            pic_params.CurrPicIdx = 0;
            pic_params.field_pic_flag = 0;
            pic_params.bottom_field_flag = 0;
            pic_params.second_field = 0;

            let status = sys::cuvidDecodePicture(self.decoder, &mut pic_params);

            // Pop context
            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuvidDecodePicture",
                    "Failed to decode picture",
                ));
            }

            // After successful decode, map and retrieve the frame
            self.retrieve_decoded_frame()?;
        }

        Ok(())
    }

    /// Retrieve the decoded frame from the decoder
    unsafe fn retrieve_decoded_frame(&mut self) -> Result<(), Error> {
        let status = unsafe { sys::cuCtxPushCurrent_v2(self.ctx) };
        if status != sys::cudaError_enum_CUDA_SUCCESS {
            return Err(Error::with_reason(
                status,
                "cuCtxPushCurrent_v2",
                "Failed to push CUDA context",
            ));
        }

        // Set up video processing parameters
        let mut proc_params: sys::CUVIDPROCPARAMS = unsafe { std::mem::zeroed() };
        proc_params.progressive_frame = 1;
        proc_params.top_field_first = 0;
        proc_params.second_field = 0;
        proc_params.output_stream = ptr::null_mut();

        // Map the decoded frame
        let mut device_ptr = 0u64;
        let mut pitch = 0u32;
        let status = unsafe {
            sys::cuvidMapVideoFrame64(
                self.decoder,
                0, // picture index
                &mut device_ptr,
                &mut pitch,
                &mut proc_params,
            )
        };

        if status != sys::cudaError_enum_CUDA_SUCCESS {
            unsafe { sys::cuCtxPopCurrent_v2(ptr::null_mut()) };
            return Err(Error::with_reason(
                status,
                "cuvidMapVideoFrame64",
                "Failed to map video frame",
            ));
        }

        // Calculate frame size (NV12 format: Y plane + UV plane)
        let y_size = pitch as usize * self.height as usize;
        let uv_size = pitch as usize * (self.height as usize / 2);
        let frame_size = y_size + uv_size;

        // Allocate host memory for the frame
        let mut host_data = vec![0u8; frame_size];

        // Copy frame data from device to host
        let status = unsafe {
            sys::cuMemcpyDtoH_v2(
                host_data.as_mut_ptr() as *mut c_void,
                device_ptr,
                frame_size,
            )
        };

        // Unmap the video frame
        unsafe { sys::cuvidUnmapVideoFrame64(self.decoder, device_ptr) };

        unsafe { sys::cuCtxPopCurrent_v2(ptr::null_mut()) };

        if status != sys::cudaError_enum_CUDA_SUCCESS {
            return Err(Error::with_reason(
                status,
                "cuMemcpyDtoH_v2",
                "Failed to copy frame data",
            ));
        }

        // Store the decoded frame
        let decoded_frame = DecodedFrame {
            width: self.width,
            height: self.height,
            data: host_data,
        };

        self.decoded_frames.push(decoded_frame);

        Ok(())
    }

    /// これ以上データが来ないことをデコーダーに伝える
    pub fn finish(&mut self) -> Result<(), Error> {
        // No-op for now since we're decoding directly
        Ok(())
    }

    /// デコード済みのフレームを取り出す
    pub fn next_frame(&mut self) -> Option<DecodedFrame> {
        if self.decoded_frames.is_empty() {
            None
        } else {
            Some(self.decoded_frames.remove(0))
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            // Destroy decoder
            if !self.decoder.is_null() {
                sys::cuvidDestroyDecoder(self.decoder);
            }
            // Destroy context
            if !self.ctx.is_null() {
                sys::cuCtxDestroy_v2(self.ctx);
            }
        }
    }
}

/// デコードされた映像フレーム (NV12 形式)
pub struct DecodedFrame {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl DecodedFrame {
    /// フレームの Y 成分のデータを返す
    pub fn y_plane(&self) -> &[u8] {
        let y_size = self.width as usize * self.height as usize;
        &self.data[..y_size]
    }

    /// フレームの UV 成分のデータを返す（NV12はインターリーブ形式）
    pub fn uv_plane(&self) -> &[u8] {
        let y_size = self.width as usize * self.height as usize;
        let uv_size = self.width as usize * (self.height as usize / 2);
        &self.data[y_size..y_size + uv_size]
    }

    /// フレームの Y 成分のストライドを返す
    pub fn y_stride(&self) -> usize {
        self.width as usize
    }

    /// フレームの UV 成分のストライドを返す
    pub fn uv_stride(&self) -> usize {
        self.width as usize
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
        let _decoder = Decoder::new_hevc().expect("Failed to initialize HEVC decoder");
        println!("HEVC decoder initialized successfully");
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

        // デコード済みフレームを取得
        let frame = decoder.next_frame().expect("No decoded frame available");

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y平面とUV平面のデータサイズを確認
        assert_eq!(frame.y_plane().len(), 640 * 480);
        assert_eq!(frame.uv_plane().len(), 640 * 240);

        // ストライドが正しいことを確認
        assert_eq!(frame.y_stride(), 640);
        assert_eq!(frame.uv_stride(), 640);

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

        // UV成分の平均値をチェック（ニュートラルは128）
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            uv_avg >= 120 && uv_avg <= 136,
            "UV average should be around 128 for neutral, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded H.265 black frame: {}x{}",
            frame.width(),
            frame.height()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);

        // フィニッシュ処理をテスト
        decoder.finish().expect("Failed to finish decoding");
    }
}
