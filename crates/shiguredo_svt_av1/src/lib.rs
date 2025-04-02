//! [Hisui] 用の [SVT-AV1] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [SVT-AV1]: https://gitlab.com/AOMediaCodec/SVT-AV1
#![warn(missing_docs)]

use std::{
    mem::MaybeUninit,
    sync::{LazyLock, Mutex},
};

mod sys;

const ENV_KEY_SVT_LOG: &'static str = "SVT_LOG";
const ENV_VALUE_SVT_LOG_LEVEL: &'static str = "1"; // 1 は error (必要に応じて調整する）

// SVT-AV1 のエンコーダー初期化処理を複数スレッドで同時に実行すると
// 大量のエラーログが出力されることがあるのでロックを使用している
static GLOBAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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

            // C++ 版では CBR を使っているけど、SVT-AV1-2.3.0 では以下のようなメッセージでエラーとなるので、
            // VBR を指定している。
            // "CBR Rate control is currently not supported for SVT_AV1_PRED_RANDOM_ACCESS, use VBR mode"
            // TODO: 後で全体的にパラメーターは見直す
            svt_config.rate_control_mode = sys::SvtAv1RcMode_SVT_AV1_RC_MODE_VBR as u8;
            svt_config.target_bit_rate = config.target_bitrate as u32;
            svt_config.force_key_frames = false;
            svt_config.source_width = config.width as u32;
            svt_config.source_height = config.height as u32;
            svt_config.frame_rate_numerator = config.fps_numerator as u32;
            svt_config.frame_rate_denominator = config.fps_denominator as u32;
            svt_config.hierarchical_levels = 0; // B フレームを無効にする
            svt_config.encoder_color_format = sys::EbColorFormat_EB_YUV420;
            svt_config.profile = 0;
            svt_config.level = 0;
            svt_config.tier = 0;

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
        }
    }
}
