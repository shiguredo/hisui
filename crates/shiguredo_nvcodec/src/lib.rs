//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/nvidia-video-codec-sdk
#![warn(missing_docs)]

use std::ffi::c_void;
use std::ptr;
use std::sync::{Arc, Mutex};

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
    parser: sys::CUvideoparser,
    ctx_lock: sys::CUvideoctxlock,
    state: Arc<Mutex<DecoderState>>,
}

struct DecoderState {
    decoder: sys::CUvideodecoder,
    width: u32,
    height: u32,
    surface_width: u32,
    surface_height: u32,
    decoded_frames: Vec<DecodedFrame>,
    ctx: sys::CUcontext,
    ctx_lock: sys::CUvideoctxlock,
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

            // Create decoder state
            let state = Arc::new(Mutex::new(DecoderState {
                decoder: ptr::null_mut(),
                width: 0,
                height: 0,
                surface_width: 0,
                surface_height: 0,
                decoded_frames: Vec::new(),
                ctx,
                ctx_lock,
            }));

            // Create video parser
            let state_ptr = Arc::into_raw(Arc::clone(&state)) as *mut c_void;

            let mut parser_params: sys::CUVIDPARSERPARAMS = std::mem::zeroed();
            parser_params.CodecType = sys::cudaVideoCodec_enum_cudaVideoCodec_HEVC;
            parser_params.ulMaxNumDecodeSurfaces = 20;
            parser_params.ulMaxDisplayDelay = 0;
            parser_params.pUserData = state_ptr;
            parser_params.pfnSequenceCallback = Some(handle_video_sequence);
            parser_params.pfnDecodePicture = Some(handle_picture_decode);
            parser_params.pfnDisplayPicture = Some(handle_picture_display);

            let mut parser = ptr::null_mut();
            let status = sys::cuvidCreateVideoParser(&mut parser, &mut parser_params);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                // Clean up the Arc reference we created
                let _ = Arc::from_raw(state_ptr as *const Mutex<DecoderState>);
                sys::cuvidCtxLockDestroy(ctx_lock);
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::with_reason(
                    status,
                    "cuvidCreateVideoParser",
                    "Failed to create video parser",
                ));
            }

            Ok(Self {
                ctx,
                parser,
                ctx_lock,
                state,
            })
        }
    }

    /// 圧縮された映像フレームをデコードする
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        if data.is_empty() {
            return Ok(());
        }

        unsafe {
            // Use the parser to decode the data
            let mut packet: sys::CUVIDSOURCEDATAPACKET = std::mem::zeroed();
            packet.payload = data.as_ptr();
            packet.payload_size = data.len() as u64;
            packet.flags = 0;
            packet.timestamp = 0;

            let status = sys::cuvidParseVideoData(self.parser, &mut packet);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuvidParseVideoData",
                    "Failed to parse video data",
                ));
            }
        }

        Ok(())
    }

    /// これ以上データが来ないことをデコーダーに伝える
    pub fn finish(&mut self) -> Result<(), Error> {
        unsafe {
            // Send end of stream packet
            let mut packet: sys::CUVIDSOURCEDATAPACKET = std::mem::zeroed();
            packet.payload = ptr::null();
            packet.payload_size = 0;
            packet.flags = sys::CUvideopacketflags_CUVID_PKT_ENDOFSTREAM as u64;
            packet.timestamp = 0;

            let status = sys::cuvidParseVideoData(self.parser, &mut packet);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuvidParseVideoData",
                    "Failed to finish decoding",
                ));
            }

            // Important: The parser processes data asynchronously. We need to ensure
            // the CUDA context is synchronized to wait for all decode operations to complete.
            let status = sys::cuCtxPushCurrent_v2(self.ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuCtxPushCurrent_v2",
                    "Failed to push CUDA context",
                ));
            }

            // Synchronize to ensure all decoding is complete
            let status = sys::cuCtxSynchronize();

            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuCtxSynchronize",
                    "Failed to synchronize CUDA context",
                ));
            }
        }
        Ok(())
    }

    /// デコード済みのフレームを取り出す
    pub fn next_frame(&mut self) -> Option<DecodedFrame> {
        let mut state = self.state.lock().unwrap();
        if state.decoded_frames.is_empty() {
            None
        } else {
            Some(state.decoded_frames.remove(0))
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            // Destroy parser first
            if !self.parser.is_null() {
                sys::cuvidDestroyVideoParser(self.parser);
            }

            // Destroy decoder
            let state = self.state.lock().unwrap();
            if !state.decoder.is_null() {
                sys::cuCtxPushCurrent_v2(self.ctx);
                sys::cuvidDestroyDecoder(state.decoder);
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
            }

            // Destroy context lock
            if !self.ctx_lock.is_null() {
                sys::cuvidCtxLockDestroy(self.ctx_lock);
            }

            // Destroy context
            if !self.ctx.is_null() {
                sys::cuCtxDestroy_v2(self.ctx);
            }
        }
    }
}

// Callback: パーサーがシーケンスヘッダーを検出した時に呼ばれる
unsafe extern "C" fn handle_video_sequence(
    user_data: *mut c_void,
    format: *mut sys::CUVIDEOFORMAT,
) -> i32 {
    if user_data.is_null() || format.is_null() {
        return 0;
    }

    let state_arc = unsafe { Arc::from_raw(user_data as *const Mutex<DecoderState>) };
    let result = (|| {
        let mut state = state_arc.lock().unwrap();
        let format = unsafe { &*format };

        // デコーダーが既に作成されている場合はスキップ
        if !state.decoder.is_null() {
            return Ok(format.min_num_decode_surfaces as i32);
        }

        // デコーダーの作成情報を設定
        let mut create_info: sys::CUVIDDECODECREATEINFO = unsafe { std::mem::zeroed() };
        create_info.CodecType = format.codec;
        create_info.ChromaFormat = format.chroma_format;
        create_info.OutputFormat = sys::cudaVideoSurfaceFormat_enum_cudaVideoSurfaceFormat_NV12;
        create_info.bitDepthMinus8 = format.bit_depth_luma_minus8 as u64;
        create_info.DeinterlaceMode = if format.progressive_sequence != 0 {
            sys::cudaVideoDeinterlaceMode_enum_cudaVideoDeinterlaceMode_Weave
        } else {
            sys::cudaVideoDeinterlaceMode_enum_cudaVideoDeinterlaceMode_Adaptive
        };
        create_info.ulNumOutputSurfaces = 2;
        create_info.ulCreationFlags =
            sys::cudaVideoCreateFlags_enum_cudaVideoCreate_PreferCUVID as u64;
        create_info.ulNumDecodeSurfaces = format.min_num_decode_surfaces as u64;
        create_info.ulWidth = format.coded_width as u64;
        create_info.ulHeight = format.coded_height as u64;
        create_info.ulMaxWidth = format.coded_width as u64;
        create_info.ulMaxHeight = format.coded_height as u64;
        create_info.ulTargetWidth = format.coded_width as u64;
        create_info.ulTargetHeight = format.coded_height as u64;

        // Use the context lock from the state (shared with parser)
        create_info.vidLock = state.ctx_lock;

        let mut decoder = ptr::null_mut();

        // Push CUDA context before creating decoder
        let status = unsafe { sys::cuCtxPushCurrent_v2(state.ctx) };
        if status != sys::cudaError_enum_CUDA_SUCCESS {
            return Err(Error::with_reason(
                status,
                "cuCtxPushCurrent_v2",
                "Failed to push CUDA context",
            ));
        }

        let status = unsafe { sys::cuvidCreateDecoder(&mut decoder, &mut create_info) };

        // Always pop context
        unsafe { sys::cuCtxPopCurrent_v2(ptr::null_mut()) };

        if status != sys::cudaError_enum_CUDA_SUCCESS {
            return Err(Error::with_reason(
                status,
                "cuvidCreateDecoder",
                "Failed to create video decoder",
            ));
        }

        state.decoder = decoder;
        state.width = (format.display_area.right - format.display_area.left) as u32;
        state.height = (format.display_area.bottom - format.display_area.top) as u32;
        state.surface_width = format.coded_width;
        state.surface_height = format.coded_height;

        Ok(format.min_num_decode_surfaces as i32)
    })();

    // Important: Don't drop the Arc, just forget our reference
    std::mem::forget(state_arc);

    match result {
        Ok(num_surfaces) => num_surfaces,
        Err(_) => 0,
    }
}

// Callback: デコードすべきピクチャーがある時に呼ばれる
unsafe extern "C" fn handle_picture_decode(
    user_data: *mut c_void,
    pic_params: *mut sys::CUVIDPICPARAMS,
) -> i32 {
    if user_data.is_null() || pic_params.is_null() {
        return 0;
    }

    let state_arc = unsafe { Arc::from_raw(user_data as *const Mutex<DecoderState>) };
    let result = (|| {
        let state = state_arc.lock().unwrap();

        if state.decoder.is_null() {
            return Err(Error::with_reason(
                1,
                "handle_picture_decode",
                "Decoder not initialized",
            ));
        }

        unsafe {
            let status = sys::cuCtxPushCurrent_v2(state.ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuCtxPushCurrent_v2",
                    "Failed to push CUDA context",
                ));
            }

            let status = sys::cuvidDecodePicture(state.decoder, pic_params);

            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuvidDecodePicture",
                    "Failed to decode picture",
                ));
            }
        }

        Ok(())
    })();

    std::mem::forget(state_arc);

    match result {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

// Callback: デコード済みフレームを表示する時に呼ばれる
unsafe extern "C" fn handle_picture_display(
    user_data: *mut c_void,
    disp_info: *mut sys::CUVIDPARSERDISPINFO,
) -> i32 {
    if user_data.is_null() || disp_info.is_null() {
        return 0;
    }

    let state_arc = unsafe { Arc::from_raw(user_data as *const Mutex<DecoderState>) };
    let result = (|| {
        let mut state = state_arc.lock().unwrap();
        let disp_info = unsafe { &*disp_info };

        if state.decoder.is_null() {
            return Err(Error::with_reason(
                1,
                "handle_picture_display",
                "Decoder not initialized",
            ));
        }

        unsafe {
            let status = sys::cuCtxPushCurrent_v2(state.ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuCtxPushCurrent_v2",
                    "Failed to push CUDA context",
                ));
            }

            // Set up video processing parameters
            let mut proc_params: sys::CUVIDPROCPARAMS = std::mem::zeroed();
            proc_params.progressive_frame = disp_info.progressive_frame;
            proc_params.top_field_first = disp_info.top_field_first;
            proc_params.second_field = (disp_info.repeat_first_field + 1) as i32;
            proc_params.output_stream = ptr::null_mut();

            // Map the decoded frame
            let mut device_ptr = 0u64;
            let mut pitch = 0u32;
            let status = sys::cuvidMapVideoFrame64(
                state.decoder,
                disp_info.picture_index,
                &mut device_ptr,
                &mut pitch,
                &mut proc_params,
            );

            if status != sys::cudaError_enum_CUDA_SUCCESS {
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::with_reason(
                    status,
                    "cuvidMapVideoFrame64",
                    "Failed to map video frame",
                ));
            }

            // Calculate frame size (NV12 format: Y plane + UV plane)
            // Note: NVDEC aligns luma height by 2
            let aligned_height = (state.surface_height + 1) & !1;
            let y_size = pitch as usize * state.height as usize;
            let uv_size = pitch as usize * (state.height as usize / 2);
            let frame_size = y_size + uv_size;

            // Allocate host memory for the frame
            let mut host_data = vec![0u8; frame_size];

            // Copy Y plane
            let status =
                sys::cuMemcpyDtoH_v2(host_data.as_mut_ptr() as *mut c_void, device_ptr, y_size);

            if status != sys::cudaError_enum_CUDA_SUCCESS {
                sys::cuvidUnmapVideoFrame64(state.decoder, device_ptr);
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::with_reason(
                    status,
                    "cuMemcpyDtoH_v2",
                    "Failed to copy Y plane data",
                ));
            }

            // Copy UV plane
            let uv_offset = pitch as u64 * aligned_height as u64;
            let status = sys::cuMemcpyDtoH_v2(
                host_data[y_size..].as_mut_ptr() as *mut c_void,
                device_ptr + uv_offset,
                uv_size,
            );

            // Unmap the video frame
            sys::cuvidUnmapVideoFrame64(state.decoder, device_ptr);
            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuMemcpyDtoH_v2",
                    "Failed to copy UV plane data",
                ));
            }

            // Store the decoded frame
            let decoded_frame = DecodedFrame {
                width: state.width,
                height: state.height,
                pitch: pitch as usize,
                data: host_data,
            };

            state.decoded_frames.push(decoded_frame);
        }

        Ok(())
    })();

    std::mem::forget(state_arc);

    match result {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

/// デコードされた映像フレーム (NV12 形式)
pub struct DecodedFrame {
    width: u32,
    height: u32,
    pitch: usize,
    data: Vec<u8>,
}

impl DecodedFrame {
    /// フレームの Y 成分のデータを返す
    pub fn y_plane(&self) -> &[u8] {
        let y_size = self.pitch * self.height as usize;
        &self.data[..y_size]
    }

    /// フレームの UV 成分のデータを返す（NV12はインターリーブ形式）
    pub fn uv_plane(&self) -> &[u8] {
        let y_size = self.pitch * self.height as usize;
        let uv_size = self.pitch * (self.height as usize / 2);
        &self.data[y_size..y_size + uv_size]
    }

    /// フレームの Y 成分のストライドを返す
    pub fn y_stride(&self) -> usize {
        self.pitch
    }

    /// フレームの UV 成分のストライドを返す
    pub fn uv_stride(&self) -> usize {
        self.pitch
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

        // UV成分の平均値をチェック（ニュートラルは128）
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            uv_avg >= 120 && uv_avg <= 136,
            "UV average should be around 128 for neutral, got {}",
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
