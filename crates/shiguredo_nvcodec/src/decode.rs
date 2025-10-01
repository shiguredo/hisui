use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::{Error, ensure_cuda_initialized, sys};

/// デコーダー
pub struct Decoder {
    ctx: sys::CUcontext,
    ctx_lock: sys::CUvideoctxlock,
    parser: sys::CUvideoparser,
    state: Box<Mutex<DecoderState>>,
    frame_rx: Receiver<Result<DecodedFrame, Error>>,
}

impl Decoder {
    /// H.265 用のデコーダーインスタンスを生成する
    pub fn new_h265() -> Result<Self, Error> {
        // CUDA ドライバーの初期化（プロセスごとに1回だけ実行される）
        ensure_cuda_initialized()?;

        unsafe {
            let mut ctx = ptr::null_mut();

            // CUDA context の初期化
            let ctx_flags = 0; // デフォルトのコンテキストフラグ
            let device_id = 0; // プライマリGPUデバイスを使用
            let status = sys::cuCtxCreate_v2(&mut ctx, ctx_flags, device_id);
            Error::check(status, "cuCtxCreate_v2", "failed to create CUDA context")?;

            // デコーダー用のコンテキストロックを作成
            let mut ctx_lock = ptr::null_mut();
            let status = sys::cuvidCtxLockCreate(&mut ctx_lock, ctx);
            Error::check(
                status,
                "cuvidCtxLockCreate",
                "failed to create context lock",
            )
            .inspect_err(|_| {
                sys::cuCtxDestroy_v2(ctx);
            })?;

            // チャンネルを作成
            let (frame_tx, frame_rx) = mpsc::channel();

            // デコーダーの状態を作成
            let state = Box::new(Mutex::new(DecoderState {
                decoder: ptr::null_mut(),
                width: 0,
                height: 0,
                surface_width: 0,
                surface_height: 0,
                frame_tx,
                ctx,
                ctx_lock,
            }));

            // 映像パーサーを作成する
            let mut parser_params: sys::CUVIDPARSERPARAMS = std::mem::zeroed();
            parser_params.CodecType = sys::cudaVideoCodec_enum_cudaVideoCodec_HEVC;
            parser_params.ulMaxNumDecodeSurfaces = 20; // TODO: 後続の PR で外から設定可能にする
            parser_params.ulMaxDisplayDelay = 0; // TODO: 後続の PR で外から設定可能にする
            parser_params.pUserData = (&*state) as *const _ as *mut c_void;
            parser_params.pfnSequenceCallback = Some(handle_video_sequence);
            parser_params.pfnDecodePicture = Some(handle_picture_decode);
            parser_params.pfnDisplayPicture = Some(handle_picture_display);

            let mut parser = ptr::null_mut();
            let status = sys::cuvidCreateVideoParser(&mut parser, &mut parser_params);
            Error::check(
                status,
                "cuvidCreateVideoParser",
                "failed to create video parser",
            )
            .inspect_err(|_| {
                sys::cuvidCtxLockDestroy(ctx_lock);
                sys::cuCtxDestroy_v2(ctx);
            })?;

            Ok(Self {
                ctx,
                ctx_lock,
                parser,
                state,
                frame_rx,
            })
        }
    }

    /// 圧縮された映像フレームをデコードする
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        // [NOTE]
        // cuvidParseVideoData は内部でデータをコピーまたは即座に処理するため、
        // このメソッドの呼び出し直後に data を破棄しても安全
        unsafe {
            let mut packet: sys::CUVIDSOURCEDATAPACKET = std::mem::zeroed();
            packet.payload = data.as_ptr();
            packet.payload_size = data.len() as u64;
            packet.flags = 0;
            packet.timestamp = 0;

            let status = sys::cuvidParseVideoData(self.parser, &mut packet);
            Error::check(status, "cuvidParseVideoData", "failed to parse video data")?;
        }

        Ok(())
    }

    /// これ以上データが来ないことをデコーダーに伝える
    pub fn finish(&mut self) -> Result<(), Error> {
        unsafe {
            // EOS をデコーダーに伝える
            let mut packet: sys::CUVIDSOURCEDATAPACKET = std::mem::zeroed();
            packet.payload = ptr::null();
            packet.payload_size = 0;
            packet.flags = sys::CUvideopacketflags_CUVID_PKT_ENDOFSTREAM as u64;
            packet.timestamp = 0;

            let status = sys::cuvidParseVideoData(self.parser, &mut packet);
            Error::check(status, "cuvidParseVideoData", "failed to finish decoding")?;

            // パーサーは非同期でデータを処理するので、
            // すべてのデコード操作が完了するまでここで待機（同期）する
            crate::with_cuda_context(self.ctx, || {
                let status = sys::cuCtxSynchronize();
                Error::check(
                    status,
                    "cuCtxSynchronize",
                    "failed to synchronize CUDA context",
                )
            })?;
        }
        Ok(())
    }

    /// デコード済みのフレームを取り出す
    pub fn next_frame(&mut self) -> Option<Result<DecodedFrame, Error>> {
        if self.state.is_poisoned() {
            return Some(Err(Error::new(
                sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
                "next_frame",
                "decoder state is poisoned (a thread panicked while holding the lock)",
            )));
        }
        self.frame_rx.try_recv().ok()
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            if !self.parser.is_null() {
                sys::cuvidDestroyVideoParser(self.parser);
            }

            // ここでロック確保に失敗してもできることはないので、成功時にだけ処理を行う
            if let Ok(state) = self.state.lock() {
                if !state.decoder.is_null() {
                    let _ = crate::with_cuda_context(self.ctx, || {
                        sys::cuvidDestroyDecoder(state.decoder);
                        Ok(())
                    });
                }
            }

            if !self.ctx_lock.is_null() {
                sys::cuvidCtxLockDestroy(self.ctx_lock);
            }

            if !self.ctx.is_null() {
                sys::cuCtxDestroy_v2(self.ctx);
            }
        }
    }
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock().ok();

        f.debug_struct("Decoder")
            .field("ctx", &format_args!("{:p}", self.ctx))
            .field("ctx_lock", &format_args!("{:p}", self.ctx_lock))
            .field("parser", &format_args!("{:p}", self.parser))
            .field(
                "decoder",
                &state.as_ref().map(|s| format!("{:p}", s.decoder)),
            )
            .field("width", &state.as_ref().map(|s| s.width))
            .field("height", &state.as_ref().map(|s| s.height))
            .field("surface_width", &state.as_ref().map(|s| s.surface_width))
            .field("surface_height", &state.as_ref().map(|s| s.surface_height))
            .finish()
    }
}

struct DecoderState {
    decoder: sys::CUvideodecoder,
    width: u32,
    height: u32,
    surface_width: u32,
    surface_height: u32,
    frame_tx: Sender<Result<DecodedFrame, Error>>,
    ctx: sys::CUcontext,
    ctx_lock: sys::CUvideoctxlock,
}

// パーサーがシーケンスヘッダーを検出した時に呼ばれるコールバック
unsafe extern "C" fn handle_video_sequence(
    user_data: *mut c_void,
    format: *mut sys::CUVIDEOFORMAT,
) -> i32 {
    if user_data.is_null() || format.is_null() {
        return 0;
    }

    let format = unsafe { &*format };
    let state = unsafe { &*(user_data as *const Mutex<DecoderState>) };
    let Ok(mut state) = state.lock() else {
        return 0;
    };

    let result = handle_video_sequence_inner(&mut state, format);
    match result {
        Ok(num_surfaces) => num_surfaces,
        Err(e) => {
            let _ = state.frame_tx.send(Err(e));
            0
        }
    }
}

fn handle_video_sequence_inner(
    state: &mut DecoderState,
    format: &sys::CUVIDEOFORMAT,
) -> Result<i32, Error> {
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
    create_info.ulCreationFlags = sys::cudaVideoCreateFlags_enum_cudaVideoCreate_PreferCUVID as u64;
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

    crate::with_cuda_context(state.ctx, || unsafe {
        Error::check(
            sys::cuvidCreateDecoder(&mut decoder, &mut create_info),
            "cuvidCreateDecoder",
            "failed to create video decoder",
        )
    })?;

    state.decoder = decoder;
    state.width = (format.display_area.right - format.display_area.left) as u32;
    state.height = (format.display_area.bottom - format.display_area.top) as u32;
    state.surface_width = format.coded_width;
    state.surface_height = format.coded_height;

    Ok(format.min_num_decode_surfaces as i32)
}

// デコードすべきピクチャーがある時に呼ばれるコールバック
unsafe extern "C" fn handle_picture_decode(
    user_data: *mut c_void,
    pic_params: *mut sys::CUVIDPICPARAMS,
) -> i32 {
    if user_data.is_null() || pic_params.is_null() {
        return 0;
    }

    let state_arc = unsafe { &*(user_data as *const Mutex<DecoderState>) };
    let result = (|| {
        let state = state_arc.lock().unwrap();

        if state.decoder.is_null() {
            let err = Error::new(1, "handle_picture_decode", "Decoder not initialized");
            let _ = state.frame_tx.send(Err(err.clone()));
            return Err(err);
        }

        crate::with_cuda_context(state.ctx, || unsafe {
            let status = sys::cuvidDecodePicture(state.decoder, pic_params);
            Error::check(status, "cuvidDecodePicture", "Failed to decode picture")
        })
        .inspect_err(|e| {
            let _ = state.frame_tx.send(Err(e.clone()));
        })?;

        Ok(())
    })();

    match result {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

// デコード済みフレームを表示する時に呼ばれるコールバック
unsafe extern "C" fn handle_picture_display(
    user_data: *mut c_void,
    disp_info: *mut sys::CUVIDPARSERDISPINFO,
) -> i32 {
    if user_data.is_null() || disp_info.is_null() {
        return 0;
    }

    let state_arc = unsafe { &*(user_data as *const Mutex<DecoderState>) };
    let result = (|| {
        let state = state_arc.lock().unwrap();
        let disp_info = unsafe { &*disp_info };

        if state.decoder.is_null() {
            let err = Error::new(1, "handle_picture_display", "Decoder not initialized");
            let _ = state.frame_tx.send(Err(err.clone()));
            return Err(err);
        }

        let decoded_frame = crate::with_cuda_context(state.ctx, || unsafe {
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

            Error::check(status, "cuvidMapVideoFrame64", "Failed to map video frame")?;

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

            if let Err(e) = Error::check(status, "cuMemcpyDtoH_v2", "Failed to copy Y plane data") {
                sys::cuvidUnmapVideoFrame64(state.decoder, device_ptr);
                return Err(e);
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

            Error::check(status, "cuMemcpyDtoH_v2", "Failed to copy UV plane data")?;

            // Store the decoded frame
            Ok(DecodedFrame {
                width: state.width,
                height: state.height,
                pitch: pitch as usize,
                data: host_data,
            })
        })
        .inspect_err(|e| {
            let _ = state.frame_tx.send(Err(e.clone()));
        })?;

        // Send through channel (ignore send errors as receiver might be dropped)
        let _ = state.frame_tx.send(Ok(decoded_frame));
        Ok(())
    })();

    match result {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

/// デコードされた映像フレーム (NV12 形式)
#[derive(Debug, Clone)]
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
    fn init_h265_decoder() {
        let _decoder = Decoder::new_h265().expect("Failed to initialize h265 decoder");
        println!("h265 decoder initialized successfully");
    }

    #[test]
    fn test_multiple_decoders() {
        // CUDA初期化が1回だけ実行されることを確認するため、複数のデコーダーを作成
        let _decoder1 = Decoder::new_h265().expect("Failed to initialize first h265 decoder");
        let _decoder2 = Decoder::new_h265().expect("Failed to initialize second h265 decoder");
        println!("Multiple h265 decoders initialized successfully");
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

        let mut decoder = Decoder::new_h265().expect("Failed to create h265 decoder");

        // デコードを実行
        decoder
            .decode(&h265_data)
            .expect("Failed to decode H.265 data");

        // フィニッシュ処理をテスト
        decoder.finish().expect("Failed to finish decoding");

        // デコード済みフレームを取得
        let frame = decoder
            .next_frame()
            .expect("No decoded frame available")
            .expect("Decoding error occurred");

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
