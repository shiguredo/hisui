//! [Hisui] 用の [SVT-AV1] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [SVT-AV1]: https://gitlab.com/AOMediaCodec/SVT-AV1
#![warn(missing_docs)]

use std::{mem::MaybeUninit, num::NonZeroUsize, sync::Mutex};

mod sys;

/// ビルド時に参照したリポジトリ URL
pub const BUILD_REPOSITORY: &str = sys::BUILD_METADATA_REPOSITORY;

/// ビルド時に参照したリポジトリのバージョン（タグ）
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

const ENV_KEY_SVT_LOG: &'static str = "SVT_LOG";
const ENV_VALUE_SVT_LOG_LEVEL: &'static str = "1"; // 1 は error (必要に応じて調整する）

// SVT-AV1 のエンコーダー初期化処理を複数スレッドで同時に実行すると
// 大量のエラーログが出力されることがあるのでロックを使用している
static GLOBAL_LOCK: Mutex<()> = Mutex::new(());

/// エラー
#[derive(Debug)]
pub struct Error {
    function: &'static str,
    code: sys::EbErrorType,
}

impl Error {
    fn check(code: sys::EbErrorType, function: &'static str) -> Result<(), Self> {
        if code == sys::EbErrorType_EB_ErrorNone {
            Ok(())
        } else {
            Err(Self { function, code })
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}() failed: code={}", self.function, self.code)
    }
}

impl std::error::Error for Error {}

/// エンコーダーに指定する設定
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// 入出力画像の幅
    pub width: usize,

    /// 入出力画像の高さ
    pub height: usize,

    /// エンコードビットレート (bps 単位)
    pub target_bitrate: usize,

    /// FPS の分子
    pub fps_numerator: usize,

    /// FPS の分母
    pub fps_denominator: usize,

    // === 品質・速度制御関連 ===
    /// エンコードプリセット (0-13, 0=最高品質・最遅, 13=最低品質・最速)
    pub enc_mode: u8,

    /// 量子化パラメータ (0-63, CQP/CRF モード時に使用)
    pub qp: Option<u8>,

    /// 最小許可QP値 (0-63)
    pub min_qp_allowed: Option<u8>,

    /// 最大許可QP値 (0-63)
    pub max_qp_allowed: Option<u8>,

    // === レート制御関連 ===
    /// レート制御モード
    pub rate_control_mode: RateControlMode,

    /// 最大ビットレート (bps 単位, Capped CRF用)
    pub max_bit_rate: Option<usize>,

    /// オーバーシュート許容率 (0-100%)
    pub over_shoot_pct: u8,

    /// アンダーシュート許容率 (0-100%)
    pub under_shoot_pct: u8,

    // === GOP・フレーム構造関連 ===
    /// イントラフレーム間隔 (-1=無制限, 0=イントラオンリー, 1以上=間隔)
    pub intra_period_length: isize,

    /// 階層レベル数 (2-5=指定値)
    pub hierarchical_levels: u8,

    /// 予測構造 (1=低遅延, 2=ランダムアクセス)
    pub pred_structure: u8,

    /// シーンチェンジ検出
    pub scene_change_detection: bool,

    /// 先読み距離 (0=無効, 1-256=フレーム数)
    pub look_ahead_distance: usize,

    // === 並列処理関連 ===
    /// スレッド数 (None=自動設定)
    pub pin_threads: Option<NonZeroUsize>,

    /// タイル列数 (None=自動)
    pub tile_columns: Option<NonZeroUsize>,

    /// タイル行数 (None=自動)
    pub tile_rows: Option<NonZeroUsize>,

    /// 対象ソケット (-1=両方, 0=ソケット0, 1=ソケット1)
    pub target_socket: isize,

    // === フィルタリング関連 ===
    /// デブロッキングフィルタ有効
    pub enable_dlf_flag: bool,

    /// CDEFレベル (-1=自動, 0=無効, 1-4=レベル)
    pub cdef_level: i8,

    /// 復元フィルタリング有効
    pub enable_restoration_filtering: bool,

    // === 高度な設定 ===
    /// テンポラルフィルタリング有効
    pub enable_tf: bool,

    /// オーバーレイフレーム有効
    pub enable_overlays: bool,

    /// フィルムグレインデノイズ強度 (0=無効, 1-50=強度)
    pub film_grain_denoise_strength: usize,

    /// TPL (Temporal Dependency Model) 有効
    pub enable_tpl_la: bool,

    /// 強制キーフレーム有効
    pub force_key_frames: bool,

    /// 統計レポート有効
    pub stat_report: bool,

    /// 再構築画像出力有効
    pub recon_enabled: bool,

    // === エンコーダー固有設定 ===
    /// ビット深度 (8, 10)
    pub encoder_bit_depth: u8,

    /// カラーフォーマット
    pub encoder_color_format: ColorFormat,

    /// プロファイル (0=Main, 1=High, 2=Professional)
    pub profile: u8,

    /// レベル (0=自動検出, 20-73=AV1レベル)
    pub level: u8,

    /// ティア (0=Main, 1=High)
    pub tier: u8,

    /// 高速デコード有効
    pub fast_decode: bool,
}

/// レート制御モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateControlMode {
    /// CQP (Constant Quantization Parameter) / CRF (Constant Rate Factor)
    CqpOrCrf,
    /// VBR (Variable Bit Rate)
    Vbr,
    /// CBR (Constant Bit Rate)
    Cbr,
}

/// カラーフォーマット
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorFormat {
    /// YUV400 (モノクロ)
    Yuv400,
    /// YUV420 (標準)
    Yuv420,
    /// YUV422
    Yuv422,
    /// YUV444
    Yuv444,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            target_bitrate: 2_000_000,
            fps_numerator: 30,
            fps_denominator: 1,
            enc_mode: 8, // バランスの良いプリセット
            qp: None,
            min_qp_allowed: None,
            max_qp_allowed: None,
            rate_control_mode: RateControlMode::Vbr,
            max_bit_rate: None,
            over_shoot_pct: 25,       // SVT-AV1のデフォルト値
            under_shoot_pct: 25,      // SVT-AV1のデフォルト値
            intra_period_length: 120, // 4秒間隔（30fps想定）
            hierarchical_levels: 5,   // 最大の階層レベル
            pred_structure: 2,        // ランダムアクセス
            scene_change_detection: true,
            look_ahead_distance: 32,
            pin_threads: None,  // 自動設定
            tile_columns: None, // 自動設定
            tile_rows: None,    // 自動設定
            target_socket: -1,  // 両方のソケット
            enable_dlf_flag: true,
            cdef_level: -1, // 自動設定
            enable_restoration_filtering: true,
            enable_tf: true,
            enable_overlays: false,
            film_grain_denoise_strength: 0, // 無効
            enable_tpl_la: true,
            force_key_frames: false,
            stat_report: false,
            recon_enabled: false,
            encoder_bit_depth: 8,
            encoder_color_format: ColorFormat::Yuv420,
            profile: 0, // Main
            level: 0,   // 自動検出
            tier: 0,    // Main
            fast_decode: false,
        }
    }
}

impl EncoderConfig {
    /// 高速エンコード用の設定
    pub fn fast_encode() -> Self {
        Self {
            enc_mode: 10,                                            // 高速プリセット
            look_ahead_distance: 16,                                 // 先読み距離を短縮
            enable_tf: false,                                        // テンポラルフィルタ無効
            enable_overlays: false,                                  // オーバーレイ無効
            scene_change_detection: false,                           // シーンチェンジ検出無効
            enable_tpl_la: false,                                    // TPL無効
            tile_columns: Some(NonZeroUsize::MIN.saturating_add(1)), // タイル分割で並列化
            tile_rows: Some(NonZeroUsize::MIN),
            hierarchical_levels: 4, // 階層レベルを制限
            ..Default::default()
        }
    }

    /// 高品質エンコード用の設定
    pub fn high_quality() -> Self {
        Self {
            enc_mode: 4,             // 高品質プリセット
            look_ahead_distance: 64, // 長い先読み
            enable_tf: true,         // テンポラルフィルタ有効
            enable_tpl_la: true,     // TPL有効
            cdef_level: 1,           // CDEF有効
            enable_restoration_filtering: true,
            over_shoot_pct: 10, // より厳しい制御
            under_shoot_pct: 10,
            ..Default::default()
        }
    }

    /// リアルタイム配信用の設定
    pub fn realtime() -> Self {
        Self {
            enc_mode: 12,                            // 最高速プリセット
            rate_control_mode: RateControlMode::Cbr, // CBR
            look_ahead_distance: 0,                  // 先読み無効
            enable_tf: false,                        // テンポラルフィルタ無効
            enable_overlays: false,                  // オーバーレイ無効
            scene_change_detection: false,           // シーンチェンジ検出無効
            enable_tpl_la: false,                    // TPL無効
            hierarchical_levels: 3,                  // 階層レベルを制限
            pred_structure: 1,                       // 低遅延
            tile_columns: Some(NonZeroUsize::MIN),
            tile_rows: Some(NonZeroUsize::MIN),
            fast_decode: true,
            over_shoot_pct: 50, // リアルタイム用途では緩い制御
            under_shoot_pct: 50,
            ..Default::default()
        }
    }
}

/// AV1 エンコーダー
#[derive(Debug)]
pub struct Encoder {
    handle: EncoderHandle,
    buffer_header: sys::EbBufferHeaderType,
    buffer: Box<sys::EbSvtIOFormat>,
    input_yuv: Vec<u8>,
    extra_data: Vec<u8>,
    frame_count: u64,
    width: usize,
    eos: bool,
}

impl Encoder {
    /// エンコーダーインスタンスを生成する
    pub fn new(config: &EncoderConfig) -> Result<Self, Error> {
        Self::with_log_level(config, ENV_VALUE_SVT_LOG_LEVEL)
    }

    fn with_log_level(config: &EncoderConfig, log_level: &str) -> Result<Self, Error> {
        let mut handle = std::ptr::null_mut();
        let buffer = MaybeUninit::<sys::EbBufferHeaderType>::zeroed();
        let buffer_format = MaybeUninit::<sys::EbSvtIOFormat>::zeroed();
        let mut svt_config = MaybeUninit::<sys::EbSvtAv1EncConfiguration>::zeroed();
        unsafe {
            // 念の為に、複数エンコーダーの同時初期化を防止するためのロックを獲得する
            let _guard = GLOBAL_LOCK.lock().inspect_err(|e| {
                // 基本はここに来ることはないはず。
                // またロック確保はあくまでも保険的なもので失敗しても致命的なものではないので、
                // ログを出すだけに留めておく
                log::warn!("failed to acquire the global lock for SVT-AV1: {e}");
            });

            // SVT-AV1 は環境変数経由でログレベルを指定するので、まず最初に設定しておく
            // この設定ができなくても致命的な問題は発生しないので、結果は単に無視する
            let _ = std::env::set_var(ENV_KEY_SVT_LOG, log_level);

            let code = sys::svt_av1_enc_init_handle(&mut handle, svt_config.as_mut_ptr());
            Error::check(code, "svt_av1_enc_init_handle")?;

            let mut handle = EncoderHandle {
                inner: handle,
                initialized: false,
            };

            let mut svt_config = svt_config.assume_init();

            // === 基本設定 ===
            svt_config.source_width = config.width as u32;
            svt_config.source_height = config.height as u32;
            svt_config.frame_rate_numerator = config.fps_numerator as u32;
            svt_config.frame_rate_denominator = config.fps_denominator as u32;
            svt_config.target_bit_rate = config.target_bitrate as u32;

            // === 品質・速度制御 ===
            svt_config.enc_mode = config.enc_mode as i8;
            if let Some(qp) = config.qp {
                svt_config.qp = qp as u32;
            }
            if let Some(min_qp) = config.min_qp_allowed {
                svt_config.min_qp_allowed = min_qp as u32;
            }
            if let Some(max_qp) = config.max_qp_allowed {
                svt_config.max_qp_allowed = max_qp as u32;
            }

            // === レート制御 ===
            svt_config.rate_control_mode = match config.rate_control_mode {
                RateControlMode::CqpOrCrf => sys::SvtAv1RcMode_SVT_AV1_RC_MODE_CQP_OR_CRF,
                RateControlMode::Vbr => sys::SvtAv1RcMode_SVT_AV1_RC_MODE_VBR,
                RateControlMode::Cbr => sys::SvtAv1RcMode_SVT_AV1_RC_MODE_CBR,
            } as u8;

            if let Some(max_bitrate) = config.max_bit_rate {
                svt_config.max_bit_rate = max_bitrate as u32;
            }
            svt_config.over_shoot_pct = config.over_shoot_pct as u32;
            svt_config.under_shoot_pct = config.under_shoot_pct as u32;

            // === GOP・フレーム構造 ===
            svt_config.intra_period_length = config.intra_period_length as i32;
            svt_config.hierarchical_levels = config.hierarchical_levels as u32;
            svt_config.pred_structure = match config.rate_control_mode {
                RateControlMode::CqpOrCrf => config.pred_structure,
                RateControlMode::Vbr => 2, // VBR の場合にはランダムアクセスのみサポート
                RateControlMode::Cbr => 1, // CBR の場合には低遅延のみサポート
            };
            svt_config.scene_change_detection = config.scene_change_detection as u32;
            svt_config.look_ahead_distance = config.look_ahead_distance as u32;

            // === 並列処理 ===
            svt_config.pin_threads = config.pin_threads.map_or(0, |v| v.get()) as u32;
            svt_config.tile_columns = config.tile_columns.map_or(0, |v| v.get()) as i32;
            svt_config.tile_rows = config.tile_rows.map_or(0, |v| v.get()) as i32;
            svt_config.target_socket = config.target_socket as i32;

            // === フィルタリング ===
            svt_config.enable_dlf_flag = config.enable_dlf_flag;
            svt_config.cdef_level = config.cdef_level as i32;
            svt_config.enable_restoration_filtering = config.enable_restoration_filtering as i32;

            // === 高度な設定 ===
            svt_config.enable_tf = config.enable_tf as u8;
            svt_config.enable_overlays = config.enable_overlays;
            svt_config.film_grain_denoise_strength = config.film_grain_denoise_strength as u32;
            svt_config.enable_tpl_la = config.enable_tpl_la as u8;
            svt_config.force_key_frames = config.force_key_frames;
            svt_config.stat_report = config.stat_report as u32;
            svt_config.recon_enabled = config.recon_enabled;

            // === エンコーダー固有設定 ===
            svt_config.encoder_bit_depth = config.encoder_bit_depth as u32;
            svt_config.encoder_color_format = match config.encoder_color_format {
                ColorFormat::Yuv400 => sys::EbColorFormat_EB_YUV400,
                ColorFormat::Yuv420 => sys::EbColorFormat_EB_YUV420,
                ColorFormat::Yuv422 => sys::EbColorFormat_EB_YUV422,
                ColorFormat::Yuv444 => sys::EbColorFormat_EB_YUV444,
            };
            svt_config.profile = config.profile as u32;
            svt_config.level = config.level as u32;
            svt_config.tier = config.tier as u32;
            svt_config.fast_decode = config.fast_decode as u8;

            // core dump する場合を予防する (C++ 版からの移植コード）
            svt_config.frame_scale_evts.start_frame_nums = std::ptr::null_mut();
            svt_config.frame_scale_evts.resize_kf_denoms = std::ptr::null_mut();
            svt_config.frame_scale_evts.resize_denoms = std::ptr::null_mut();

            let code = sys::svt_av1_enc_set_parameter(handle.inner, &mut svt_config);
            Error::check(code, "svt_av1_enc_set_parameter")?;

            let code = sys::svt_av1_enc_init(handle.inner);
            Error::check(code, "svt_av1_enc_init")?;
            handle.initialized = true;

            let mut buffer_header = buffer.assume_init();
            let mut buffer = Box::new(buffer_format.assume_init());
            buffer_header.p_buffer = (&raw mut *buffer).cast();
            buffer_header.size = size_of_val(&buffer_header) as u32;
            buffer_header.p_app_private = std::ptr::null_mut();
            buffer_header.pic_type = sys::EbAv1PictureType_EB_AV1_INVALID_PICTURE;
            buffer_header.metadata = std::ptr::null_mut();

            let y_size = config.height * config.width;
            let u_size = config.height.div_ceil(2) * config.width.div_ceil(2);
            let v_size = config.height.div_ceil(2) * config.width.div_ceil(2);
            let mut input_yuv = vec![0; y_size + u_size + v_size];

            buffer.luma = input_yuv.as_mut_ptr();
            buffer.cb = input_yuv.as_mut_ptr().add(y_size);
            buffer.cr = input_yuv.as_mut_ptr().add(y_size + u_size);
            buffer_header.n_filled_len = input_yuv.len() as u32;

            let mut stream_header = std::ptr::null_mut();
            let code = sys::svt_av1_enc_stream_header(handle.inner, &mut stream_header);
            Error::check(code, "svt_av1_enc_stream_header")?;

            let extra_data = std::slice::from_raw_parts(
                (*stream_header).p_buffer,
                (*stream_header).n_filled_len as usize,
            )
            .to_vec();

            let code = sys::svt_av1_enc_stream_header_release(stream_header);
            Error::check(code, "svt_av1_enc_stream_header_release")?;

            Ok(Self {
                handle,
                buffer_header,
                buffer,
                input_yuv,
                extra_data,
                frame_count: 0,
                width: config.width,
                eos: false,
            })
        }
    }

    /// MP4 の av01 ボックスに格納するデコーダー向けの情報
    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    /// I420 形式の画像データをエンコードする
    ///
    /// エンコード結果は [`Encoder::next_frame()`] で取得できる
    ///
    /// なお `y` のストライドは入力フレームの幅と等しいことが前提
    ///
    /// また B フレームは扱わない前提（つまり入力フレームと出力フレームの順番が一致する）
    pub fn encode(&mut self, y: &[u8], u: &[u8], v: &[u8]) -> Result<(), Error> {
        if self.input_yuv.len() != y.len() + u.len() + v.len() {
            // 入力データのサイズが不正
            Error::check(
                sys::EbErrorType_EB_ErrorBadParameter,
                "shiguredo_svt_av1::Encoder::encode",
            )?;
        }

        self.input_yuv[..y.len()].copy_from_slice(y);
        self.input_yuv[y.len()..][..u.len()].copy_from_slice(u);
        self.input_yuv[y.len() + u.len()..][..v.len()].copy_from_slice(v);

        self.buffer_header.flags = 0;
        self.buffer_header.p_app_private = std::ptr::null_mut();
        self.buffer_header.pts = self.frame_count as i64;
        self.buffer_header.pic_type = sys::EbAv1PictureType_EB_AV1_INVALID_PICTURE;
        self.buffer_header.metadata = std::ptr::null_mut();
        self.buffer.y_stride = self.width as u32;
        self.buffer.cb_stride = self.width.div_ceil(2) as u32;
        self.buffer.cr_stride = self.width.div_ceil(2) as u32;

        let code =
            unsafe { sys::svt_av1_enc_send_picture(self.handle.inner, &mut self.buffer_header) };
        Error::check(code, "svt_av1_enc_send_picture")?;

        self.frame_count += 1;
        Ok(())
    }

    /// これ以上データが来ないことをエンコーダーに伝える
    ///
    /// 残りのエンコード結果は [`Encoder::next_frame()`] で取得できる
    pub fn finish(&mut self) -> Result<(), Error> {
        self.buffer_header.flags = sys::EB_BUFFERFLAG_EOS;
        self.buffer_header.pic_type = sys::EbAv1PictureType_EB_AV1_INVALID_PICTURE;
        let code =
            unsafe { sys::svt_av1_enc_send_picture(self.handle.inner, &mut self.buffer_header) };
        Error::check(code, "svt_av1_enc_send_picture")?;
        self.eos = true;

        Ok(())
    }

    /// エンコード済みのフレームを取り出す
    ///
    /// [`Encoder::encode()`] や [`Encoder::finish()`] の後には、
    /// このメソッドを、結果が `None` になるまで呼び出し続ける必要がある
    pub fn next_frame(&mut self) -> Result<Option<EncodedFrame>, Error> {
        let mut output = std::ptr::null_mut();
        let pic_send_done = self.eos as u8;
        let code =
            unsafe { sys::svt_av1_enc_get_packet(self.handle.inner, &mut output, pic_send_done) };
        if code == sys::EbErrorType_EB_NoErrorEmptyQueue {
            return Ok(None);
        }
        Error::check(code, "svt_av1_enc_get_packet")?;

        let frame = unsafe { EncodedFrame(&mut *output) };
        if (frame.0.flags & sys::EB_BUFFERFLAG_EOS) != 0 {
            Ok(None)
        } else {
            Ok(Some(frame))
        }
    }
}

unsafe impl Send for Encoder {}

#[derive(Debug)]
struct EncoderHandle {
    inner: *mut sys::EbComponentType,
    initialized: bool,
}

impl Drop for EncoderHandle {
    fn drop(&mut self) {
        unsafe {
            if self.initialized {
                sys::svt_av1_enc_deinit(self.inner);
            }
            sys::svt_av1_enc_deinit_handle(self.inner);
        }
    }
}

/// エンコードされた映像フレーム
#[derive(Debug)]
pub struct EncodedFrame<'a>(&'a mut sys::EbBufferHeaderType);

impl EncodedFrame<'_> {
    /// 圧縮データ
    pub fn data(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.0.p_buffer, self.0.n_filled_len as usize) }
    }

    /// キーフレームかどうか
    pub fn is_keyframe(&self) -> bool {
        matches!(
            self.0.pic_type,
            sys::EbAv1PictureType_EB_AV1_KEY_PICTURE
                | sys::EbAv1PictureType_EB_AV1_INTRA_ONLY_PICTURE
        )
    }
}

impl Drop for EncodedFrame<'_> {
    fn drop(&mut self) {
        unsafe {
            sys::svt_av1_enc_release_out_buffer(&mut (self.0 as *mut _));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_encoder() {
        // OK
        let config = encoder_config();
        assert!(Encoder::new(&config).is_ok());

        // NG (どうしても SVT-AV1 のエラーログが出てしまい紛らわしいので、エラーログを抑制するようにしている）
        let mut config = encoder_config();
        config.fps_denominator = 0;
        assert!(Encoder::with_log_level(&config, "0").is_err());
    }

    #[test]
    fn encode_black() {
        let config = encoder_config();
        let mut encoder = Encoder::new(&config).expect("failed to create");
        let mut encoded_count = 0;

        let size = config.width * config.height;
        let y = vec![0; size];
        let u = vec![0; size / 4];
        let v = vec![0; size / 4];

        encoder.encode(&y, &u, &v).expect("failed to encode");
        while let Ok(Some(_)) = encoder.next_frame() {
            encoded_count += 1;
        }

        // 一フレームだけ処理すると SVT-AV1 が `--avif 1` を使うようにエラーログを出すので
        // それを防止するために二フレーム目も与えている
        encoder.encode(&y, &u, &v).expect("failed to encode");
        while let Ok(Some(_)) = encoder.next_frame() {
            encoded_count += 1;
        }

        encoder.finish().expect("failed to finish");
        while let Ok(Some(_)) = encoder.next_frame() {
            encoded_count += 1;
        }

        assert_eq!(encoded_count, 2);
    }

    fn encoder_config() -> EncoderConfig {
        EncoderConfig {
            target_bitrate: 1000_000,
            width: 320,
            height: 320,
            fps_numerator: 1,
            fps_denominator: 1,
            ..Default::default()
        }
    }
}
