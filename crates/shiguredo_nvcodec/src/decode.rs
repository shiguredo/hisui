// TODO: VP8 / VP9 のデコードに対応する
use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::{CudaLibrary, Error, sys};

/// デコーダーの設定
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    /// 使用する GPU デバイスの ID (デフォルト: 0)
    pub device_id: i32,

    /// デコード用サーフェスの最大数 (デフォルト: 20)
    pub max_num_decode_surfaces: u32,

    /// 表示遅延 (デフォルト: 0 = 低遅延)
    pub max_display_delay: u32,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            device_id: 0,
            max_num_decode_surfaces: 20,
            max_display_delay: 0,
        }
    }
}

/// デコーダー
pub struct Decoder {
    lib: CudaLibrary,
    ctx: sys::CUcontext,
    ctx_lock: sys::CUvideoctxlock,
    parser: sys::CUvideoparser,
    state: Box<Mutex<DecoderState>>,
    frame_rx: Receiver<Result<DecodedFrame, Error>>,
}

impl Decoder {
    /// H.264 用のデコーダーインスタンスを生成する
    pub fn new_h264(config: DecoderConfig) -> Result<Self, Error> {
        Self::new_with_codec(sys::cudaVideoCodec_enum_cudaVideoCodec_H264, config)
    }

    /// H.265 用のデコーダーインスタンスを生成する
    pub fn new_h265(config: DecoderConfig) -> Result<Self, Error> {
        Self::new_with_codec(sys::cudaVideoCodec_enum_cudaVideoCodec_HEVC, config)
    }

    /// AV1 用のデコーダーインスタンスを生成する
    pub fn new_av1(config: DecoderConfig) -> Result<Self, Error> {
        Self::new_with_codec(sys::cudaVideoCodec_enum_cudaVideoCodec_AV1, config)
    }

    /// VP8 用のデコーダーインスタンスを生成する
    pub fn new_vp8(config: DecoderConfig) -> Result<Self, Error> {
        Self::new_with_codec(sys::cudaVideoCodec_enum_cudaVideoCodec_VP8, config)
    }

    /// VP9 用のデコーダーインスタンスを生成する
    pub fn new_vp9(config: DecoderConfig) -> Result<Self, Error> {
        Self::new_with_codec(sys::cudaVideoCodec_enum_cudaVideoCodec_VP9, config)
    }

    /// 指定されたコーデックタイプでデコーダーインスタンスを生成する
    fn new_with_codec(
        codec_type: sys::cudaVideoCodec,
        config: DecoderConfig,
    ) -> Result<Self, Error> {
        unsafe {
            let lib = CudaLibrary::load()?;

            let mut ctx = ptr::null_mut();

            // CUDA context の初期化
            let ctx_flags = 0; // デフォルトのコンテキストフラグ
            lib.cu_ctx_create(&mut ctx, ctx_flags, config.device_id)?;

            let ctx_guard = crate::ReleaseGuard::new(|| {
                let _ = lib.cu_ctx_destroy(ctx);
            });

            // デコーダー用のコンテキストロックを作成
            let mut ctx_lock = ptr::null_mut();
            lib.cuvid_ctx_lock_create(&mut ctx_lock, ctx)?;

            let ctx_lock_guard = crate::ReleaseGuard::new(|| {
                let _ = lib.cuvid_ctx_lock_destroy(ctx_lock);
            });

            // チャンネルを作成
            let (frame_tx, frame_rx) = mpsc::channel();

            // デコーダーの状態を作成
            let state = Box::new(Mutex::new(DecoderState {
                lib: lib.clone(),
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
            parser_params.CodecType = codec_type;
            parser_params.ulMaxNumDecodeSurfaces = config.max_num_decode_surfaces;
            parser_params.ulMaxDisplayDelay = config.max_display_delay;
            parser_params.pUserData = (&*state) as *const _ as *mut c_void;
            parser_params.pfnSequenceCallback = Some(handle_video_sequence);
            parser_params.pfnDecodePicture = Some(handle_picture_decode);
            parser_params.pfnDisplayPicture = Some(handle_picture_display);

            let mut parser = ptr::null_mut();
            lib.cuvid_create_video_parser(&mut parser, &mut parser_params)?;

            // 成功したのでクリーンアップをキャンセル
            ctx_guard.cancel();
            ctx_lock_guard.cancel();

            Ok(Self {
                lib,
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

            self.lib.cuvid_parse_video_data(self.parser, &mut packet)?;
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

            self.lib.cuvid_parse_video_data(self.parser, &mut packet)?;

            // パーサーは非同期でデータを処理するので、
            // すべてのデコード操作が完了するまでここで待機（同期）する
            self.lib
                .with_context(self.ctx, || self.lib.cu_ctx_synchronize())?;
        }
        Ok(())
    }

    /// デコード済みのフレームを取り出す
    pub fn next_frame(&mut self) -> Result<Option<DecodedFrame>, Error> {
        if self.state.is_poisoned() {
            return Err(Error::new(
                sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
                "next_frame",
                "decoder state is poisoned (a thread panicked while holding the lock)",
            ));
        }
        self.frame_rx.try_recv().ok().transpose()
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if !self.parser.is_null() {
            let _ = self.lib.cuvid_destroy_video_parser(self.parser);
        }

        // ここでロック確保に失敗してもできることはないので、成功時にだけ処理を行う
        if let Ok(state) = self.state.lock()
            && !state.decoder.is_null()
        {
            let _ = self
                .lib
                .with_context(self.ctx, || self.lib.cuvid_destroy_decoder(state.decoder));
        }

        if !self.ctx_lock.is_null() {
            let _ = self.lib.cuvid_ctx_lock_destroy(self.ctx_lock);
        }

        if !self.ctx.is_null() {
            let _ = self.lib.cu_ctx_destroy(self.ctx);
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

unsafe impl Send for Decoder {}

struct DecoderState {
    lib: CudaLibrary,
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
        // このケースは next_frame() の中でハンドリングされているので、ここでは何もする必要がない
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
    create_info.ulNumOutputSurfaces = 2; // 出力サーフェスの数（ダブルバッファリング用に2を指定）
    create_info.ulCreationFlags = sys::cudaVideoCreateFlags_enum_cudaVideoCreate_PreferCUVID as u64; // CUVID ハードウェアデコーダーの使用を優先するフラグ
    create_info.ulNumDecodeSurfaces = format.min_num_decode_surfaces as u64;
    create_info.ulWidth = format.coded_width as u64;
    create_info.ulHeight = format.coded_height as u64;
    create_info.ulMaxWidth = format.coded_width as u64;
    create_info.ulMaxHeight = format.coded_height as u64;
    create_info.ulTargetWidth = format.coded_width as u64;
    create_info.ulTargetHeight = format.coded_height as u64;

    // パーサーと共有するコンテキストロックを使用
    create_info.vidLock = state.ctx_lock;

    state.lib.with_context(state.ctx, || {
        state
            .lib
            .cuvid_create_decoder(&mut state.decoder, &mut create_info)
    })?;
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

    let state = unsafe { &*(user_data as *const Mutex<DecoderState>) };
    let Ok(mut state) = state.lock() else {
        // このケースは next_frame() の中でハンドリングされているので、ここでは何もする必要がない
        return 0;
    };

    let result = handle_picture_decode_inner(&mut state, unsafe { &*pic_params });
    match result {
        Ok(_) => 1,
        Err(e) => {
            let _ = state.frame_tx.send(Err(e));
            0
        }
    }
}

fn handle_picture_decode_inner(
    state: &mut DecoderState,
    pic_params: &sys::CUVIDPICPARAMS,
) -> Result<(), Error> {
    if state.decoder.is_null() {
        return Err(Error::new(
            sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
            "handle_picture_decode",
            "decoder not initialized",
        ));
    }

    state.lib.with_context(state.ctx, || {
        state
            .lib
            .cuvid_decode_picture(state.decoder, pic_params as *const _ as *mut _)
    })?;

    Ok(())
}

// デコード済みフレームを表示する時に呼ばれるコールバック
unsafe extern "C" fn handle_picture_display(
    user_data: *mut c_void,
    disp_info: *mut sys::CUVIDPARSERDISPINFO,
) -> i32 {
    if user_data.is_null() || disp_info.is_null() {
        return 0;
    }

    let state = unsafe { &*(user_data as *const Mutex<DecoderState>) };
    let Ok(state) = state.lock() else {
        // このケースは next_frame() の中でハンドリングされているので、ここでは何もする必要がない
        return 0;
    };

    let result = handle_picture_display_inner(&state, unsafe { &*disp_info });
    match result {
        Ok(_) => 1,
        Err(e) => {
            let _ = state.frame_tx.send(Err(e));
            0
        }
    }
}

fn handle_picture_display_inner(
    state: &DecoderState,
    disp_info: &sys::CUVIDPARSERDISPINFO,
) -> Result<(), Error> {
    if state.decoder.is_null() {
        return Err(Error::new(
            sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
            "handle_picture_display",
            "decoder not initialized",
        ));
    }

    let decoded_frame = state.lib.with_context(state.ctx, || unsafe {
        // ビデオ処理パラメーターを設定
        let mut proc_params: sys::CUVIDPROCPARAMS = std::mem::zeroed();
        proc_params.progressive_frame = disp_info.progressive_frame;
        proc_params.top_field_first = disp_info.top_field_first;
        proc_params.second_field = disp_info.repeat_first_field + 1;
        proc_params.output_stream = ptr::null_mut();

        // デコード済みフレームをマップ
        let mut device_ptr = 0u64;
        let mut pitch = 0u32;
        state.lib.cuvid_map_video_frame(
            state.decoder,
            disp_info.picture_index,
            &mut device_ptr,
            &mut pitch,
            &mut proc_params,
        )?;

        // 確実にフレームをアンマップするためのガードを作成
        let _unmap_guard = crate::ReleaseGuard::new(|| {
            let _ = state.lib.cuvid_unmap_video_frame(state.decoder, device_ptr);
        });

        // フレームサイズを計算 (NV12 形式: Y プレーン + UV プレーン)
        // 注意: NVDEC は高さを 2 でアライメントする
        let aligned_height = (state.surface_height + 1) & !1;
        let y_size = pitch as usize * state.height as usize;
        let uv_size = pitch as usize * (state.height as usize / 2);
        let frame_size = y_size + uv_size;

        // フレーム用のホストメモリを割り当て
        let mut host_data = vec![0u8; frame_size];

        // Y プレーンをコピー
        state
            .lib
            .cu_memcpy_d_to_h(host_data.as_mut_ptr() as *mut c_void, device_ptr, y_size)?;

        // UV プレーンをコピー
        let uv_offset = pitch as u64 * aligned_height as u64;
        state.lib.cu_memcpy_d_to_h(
            host_data[y_size..].as_mut_ptr() as *mut c_void,
            device_ptr + uv_offset,
            uv_size,
        )?;

        // デコード済みフレームを作成
        Ok(DecodedFrame {
            width: state.width,
            height: state.height,
            pitch: pitch as usize,
            data: host_data,
        })
    })?;

    // チャンネル経由で送信 (受信側が破棄されている場合の送信エラーは無視)
    let _ = state.frame_tx.send(Ok(decoded_frame));

    Ok(())
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
        let _decoder =
            Decoder::new_h265(DecoderConfig::default()).expect("Failed to initialize h265 decoder");
        println!("h265 decoder initialized successfully");
    }

    #[test]
    fn init_h264_decoder() {
        let _decoder =
            Decoder::new_h264(DecoderConfig::default()).expect("Failed to initialize h264 decoder");
        println!("h264 decoder initialized successfully");
    }

    #[test]
    fn init_av1_decoder() {
        let _decoder =
            Decoder::new_av1(DecoderConfig::default()).expect("Failed to initialize av1 decoder");
        println!("av1 decoder initialized successfully");
    }

    #[test]
    fn init_vp8_decoder() {
        let _decoder =
            Decoder::new_vp8(DecoderConfig::default()).expect("Failed to initialize vp8 decoder");
        println!("vp8 decoder initialized successfully");
    }

    #[test]
    fn init_vp9_decoder() {
        let _decoder =
            Decoder::new_vp9(DecoderConfig::default()).expect("Failed to initialize vp9 decoder");
        println!("vp9 decoder initialized successfully");
    }

    #[test]
    fn test_multiple_decoders() {
        // CUDA初期化が1回だけ実行されることを確認するため、複数のデコーダーを作成
        let _decoder1 = Decoder::new_h265(DecoderConfig::default())
            .expect("Failed to initialize first h265 decoder");
        let _decoder2 = Decoder::new_h265(DecoderConfig::default())
            .expect("Failed to initialize second h265 decoder");
        println!("Multiple h265 decoders initialized successfully");
    }

    #[test]
    fn test_decode_h265_black_frame() {
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

        let mut decoder =
            Decoder::new_h265(DecoderConfig::default()).expect("Failed to create h265 decoder");

        // デコードを実行
        decoder
            .decode(&h265_data)
            .expect("Failed to decode H.265 data");

        // フィニッシュ処理をテスト
        decoder.finish().expect("Failed to finish decoding");

        // デコード済みフレームを取得
        let frame = decoder
            .next_frame()
            .expect("Decoding error occurred")
            .expect("No decoded frame available");

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

    #[test]
    fn test_decode_h264_black_frame() {
        // H.264の黒フレームデータ (NAL units with size prefix)
        let sps = vec![
            103, 100, 0, 30, 172, 217, 64, 160, 61, 176, 17, 0, 0, 3, 0, 1, 0, 0, 3, 0, 50, 15, 22,
            45, 150,
        ];
        let pps = vec![104, 235, 227, 203, 34, 192];
        let frame_data = vec![
            101, 136, 132, 0, 43, 255, 254, 246, 115, 124, 10, 107, 109, 176, 149, 46, 5, 118, 247,
            102, 163, 229, 208, 146, 229, 251, 16, 96, 250, 208, 0, 0, 3, 0, 0, 3, 0, 0, 16, 15,
            210, 222, 245, 204, 98, 91, 229, 32, 0, 0, 9, 216, 2, 56, 13, 16, 118, 133, 116, 69,
            196, 32, 71, 6, 120, 150, 16, 161, 210, 50, 128, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0,
            0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 37, 225,
        ];

        // NALユニットを結合（Annex B形式: start code 0x00000001 を使用）
        let mut h264_data = Vec::new();
        let start_code = [0u8, 0, 0, 1];

        // SPS
        h264_data.extend_from_slice(&start_code);
        h264_data.extend_from_slice(&sps);

        // PPS
        h264_data.extend_from_slice(&start_code);
        h264_data.extend_from_slice(&pps);

        // Frame data
        h264_data.extend_from_slice(&start_code);
        h264_data.extend_from_slice(&frame_data);

        let mut decoder =
            Decoder::new_h264(DecoderConfig::default()).expect("Failed to create h264 decoder");

        // デコードを実行
        decoder
            .decode(&h264_data)
            .expect("Failed to decode H.264 data");

        // フィニッシュ処理をテスト
        decoder.finish().expect("Failed to finish decoding");

        // デコード済みフレームを取得
        let frame = decoder
            .next_frame()
            .expect("Decoding error occurred")
            .expect("No decoded frame available");

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y平面とUV平面のデータサイズを確認
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
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            uv_avg >= 70 && uv_avg <= 140,
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded H.264 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);
    }

    #[test]
    fn test_decode_av1_black_frame() {
        // AV1の黒フレームデータ (OBU format)
        // OBU_TYPE=1 (sequence header) と OBU_TYPE=6 (frame) を含む
        let av1_data = vec![
            // TYPE=1 (Sequence Header OBU)
            10, 11, 0, 0, 0, 36, 196, 255, 223, 63, 254, 96, 16, // TYPE=6 (Frame OBU)
            50, 35, 16, 0, 144, 0, 0, 0, 160, 0, 0, 128, 1, 197, 120, 80, 103, 179, 239, 241, 100,
            76, 173, 116, 93, 183, 31, 101, 221, 87, 90, 233, 219, 28, 199, 243, 128,
        ];

        let mut decoder =
            Decoder::new_av1(DecoderConfig::default()).expect("Failed to create av1 decoder");

        // デコードを実行
        decoder
            .decode(&av1_data)
            .expect("Failed to decode AV1 data");

        // フィニッシュ処理をテスト
        decoder.finish().expect("Failed to finish decoding");

        // デコード済みフレームを取得
        let frame = decoder
            .next_frame()
            .expect("Decoding error occurred")
            .expect("No decoded frame available");

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y平面とUV平面のデータサイズを確認
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
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            uv_avg >= 70 && uv_avg <= 140,
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded AV1 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);
    }

    #[test]
    fn test_decode_vp8_black_frame() {
        // VP8の黒フレームデータ
        let vp8_data = vec![
            80, 66, 0, 157, 1, 42, 128, 2, 224, 1, 2, 199, 8, 133, 133, 136, 153, 132, 136, 15, 2,
            0, 6, 22, 4, 247, 6, 129, 100, 159, 107, 219, 155, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 55, 128, 254,
            250, 215, 128,
        ];

        let mut decoder =
            Decoder::new_vp8(DecoderConfig::default()).expect("Failed to create vp8 decoder");

        // デコードを実行
        decoder
            .decode(&vp8_data)
            .expect("Failed to decode VP8 data");

        // フィニッシュ処理をテスト
        decoder.finish().expect("Failed to finish decoding");

        // デコード済みフレームを取得
        let frame = decoder
            .next_frame()
            .expect("Decoding error occurred")
            .expect("No decoded frame available");

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y平面とUV平面のデータサイズを確認
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
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            uv_avg >= 70 && uv_avg <= 140,
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded VP8 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);
    }

    #[test]
    fn test_decode_vp9_black_frame() {
        // VP9の黒フレームデータ
        let vp9_data = vec![
            130, 73, 131, 66, 0, 39, 240, 29, 246, 0, 56, 36, 28, 24, 74, 16, 0, 80, 97, 246, 58,
            246, 128, 92, 209, 238, 0, 0, 0, 0, 0, 20, 103, 26, 154, 224, 98, 35, 126, 68, 120,
            240, 227, 199, 143, 30, 28, 238, 113, 218, 24, 0, 103, 26, 154, 224, 98, 35, 126, 68,
            120, 240, 227, 199, 143, 30, 28, 238, 113, 218, 24, 0,
        ];

        let mut decoder =
            Decoder::new_vp9(DecoderConfig::default()).expect("Failed to create vp9 decoder");

        // デコードを実行
        decoder
            .decode(&vp9_data)
            .expect("Failed to decode VP9 data");

        // フィニッシュ処理をテスト
        decoder.finish().expect("Failed to finish decoding");

        // デコード済みフレームを取得
        let frame = decoder
            .next_frame()
            .expect("Decoding error occurred")
            .expect("No decoded frame available");

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y平面とUV平面のデータサイズを確認
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
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            uv_avg >= 70 && uv_avg <= 140,
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded VP9 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);
    }
}
