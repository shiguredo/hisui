//! [Hisui] 用の [libvpx] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [libvpx]: https://github.com/webmproject/libvpx
#![warn(missing_docs)]

use std::{
    ffi::{c_int, c_uint, CStr},
    mem::MaybeUninit,
};

mod sys;

/// エラー
#[derive(Debug)]
pub struct Error {
    code: sys::vpx_codec_err_t,
    function: &'static str,
    reason: Option<&'static str>,
}

impl Error {
    fn check(code: sys::vpx_codec_err_t, function: &'static str) -> Result<(), Self> {
        if code == sys::vpx_codec_err_t_VPX_CODEC_OK {
            Ok(())
        } else {
            Err(Self {
                code,
                function,
                reason: None,
            })
        }
    }

    fn with_reason(
        code: sys::vpx_codec_err_t,
        function: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            code,
            function,
            reason: Some(reason),
        }
    }

    fn reason(&self) -> Option<&str> {
        if self.reason.is_some() {
            return self.reason;
        }

        let reason = unsafe { sys::vpx_codec_err_to_string(self.code) };
        if reason.is_null() {
            None
        } else {
            unsafe { CStr::from_ptr(reason) }.to_str().ok()
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(reason) = self.reason() {
            write!(
                f,
                "{}() failed: code={}, reason={}",
                self.function, self.code, reason
            )
        } else {
            write!(f, "{}() failed: code={}", self.function, self.code)
        }
    }
}

impl std::error::Error for Error {}

/// VP8 / VP9 デコーダー
pub struct Decoder {
    ctx: sys::vpx_codec_ctx,
    iter: sys::vpx_codec_iter_t,
}

impl Decoder {
    /// VP8 用のデコーダーインスタンスを生成する
    pub fn new_vp8() -> Result<Self, Error> {
        unsafe {
            let iface = sys::vpx_codec_vp8_dx();
            Self::new(iface)
        }
    }

    /// VP9 用のデコーダーインスタンスを生成する
    pub fn new_vp9() -> Result<Self, Error> {
        unsafe {
            let iface = sys::vpx_codec_vp9_dx();
            Self::new(iface)
        }
    }

    fn new(iface: *const sys::vpx_codec_iface) -> Result<Self, Error> {
        let mut ctx = MaybeUninit::<sys::vpx_codec_ctx>::zeroed();
        unsafe {
            let code = sys::vpx_codec_dec_init_ver(
                ctx.as_mut_ptr(),
                iface,
                std::ptr::null(), // cfg
                0,                // flags
                sys::VPX_DECODER_ABI_VERSION as i32,
            );
            Error::check(code, "vpx_codec_dec_init_ver")?;

            let ctx = ctx.assume_init();
            Ok(Self {
                ctx,
                iter: std::ptr::null(),
            })
        }
    }

    /// 圧縮された映像フレームをデコードする
    ///
    /// デコード結果は [`Decoder::next_frame()`] で取得できる
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        if !self.iter.is_null() {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_ERROR,
                "shiguredo_libvpx::Decoder::decode",
                "still need to call shiguredo_libvpx::Decoder::next_frame()",
            ));
        }

        let code = unsafe {
            sys::vpx_codec_decode(
                &mut self.ctx,
                data.as_ptr(),
                data.len() as c_uint,
                std::ptr::null_mut(), // user_priv
                0, // deadline (ドキュメントによると、値は無視されるので常に 0 を指定しろとのこと）
            )
        };
        Error::check(code, "vpx_codec_decode")?;
        Ok(())
    }

    /// これ以上データが来ないことをデコーダーに伝える
    ///
    /// 残りのデコード結果は [`Decoder::next_frame()`] で取得できる
    pub fn finish(&mut self) -> Result<(), Error> {
        if !self.iter.is_null() {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_ERROR,
                "shiguredo_libvpx::Decoder::finish",
                "still need to call shiguredo_libvpx::Decoder::next_frame()",
            ));
        }

        let code = unsafe {
            sys::vpx_codec_decode(
                &mut self.ctx,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
            )
        };
        Error::check(code, "vpx_codec_decode")?;
        Ok(())
    }

    /// デコード済みのフレームを取り出す
    ///
    /// [`Decoder::decode()`] や [`Decoder::finish()`] の後には、
    /// このメソッドを、結果が `None` になるまで呼び出し続ける必要がある
    pub fn next_frame(&mut self) -> Option<DecodedFrame> {
        unsafe {
            let image = sys::vpx_codec_get_frame(&mut self.ctx, &mut self.iter);
            if image.is_null() {
                self.iter = std::ptr::null();
                return None;
            }
            let image = &*image;

            // 画像フォーマットは I420 である前提
            assert_eq!(image.fmt, sys::vpx_img_fmt_VPX_IMG_FMT_I420);

            Some(DecodedFrame(image))
        }
    }
}

unsafe impl Send for Decoder {}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            sys::vpx_codec_destroy(&mut self.ctx);
        }
    }
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder").finish_non_exhaustive()
    }
}

/// デコードされた映像フレーム (I420 形式)
pub struct DecodedFrame<'a>(&'a sys::vpx_image);

impl DecodedFrame<'_> {
    /// フレームの Y 成分のデータを返す
    pub fn y_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(self.0.planes[0], self.0.d_h as usize * self.y_stride())
        }
    }

    /// フレームの U 成分のデータを返す
    pub fn u_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.0.planes[1],
                self.0.d_h.div_ceil(2) as usize * self.u_stride(),
            )
        }
    }

    /// フレームの V 成分のデータを返す
    pub fn v_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.0.planes[2],
                self.0.d_h.div_ceil(2) as usize * self.v_stride(),
            )
        }
    }

    /// フレームの Y 成分のストライドを返す
    pub fn y_stride(&self) -> usize {
        self.0.stride[0] as usize
    }

    /// フレームの U 成分のストライドを返す
    pub fn u_stride(&self) -> usize {
        self.0.stride[1] as usize
    }

    /// フレームの V 成分のストライドを返す
    pub fn v_stride(&self) -> usize {
        self.0.stride[2] as usize
    }

    /// フレームの幅を返す
    pub fn width(&self) -> usize {
        self.0.d_w as usize
    }

    /// フレームの高さを返す
    pub fn height(&self) -> usize {
        self.0.d_h as usize
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

    /// libvpx に指定する品質調整用パラメーター
    pub min_quantizer: usize,

    /// libvpx に指定する品質調整用パラメーター
    pub max_quantizer: usize,

    /// libvpx に指定する品質調整用パラメーター
    pub cq_level: usize,
}

/// VP8 / VP9 エンコーダー
pub struct Encoder {
    ctx: sys::vpx_codec_ctx,
    img: sys::vpx_image,
    iter: sys::vpx_codec_iter_t,
    frame_count: usize,
    y_size: usize,
    u_size: usize,
    v_size: usize,
}

impl Encoder {
    /// VP8 用のエンコーダーインスタンスを生成する
    pub fn new_vp8(config: &EncoderConfig) -> Result<Self, Error> {
        let mut cfg = MaybeUninit::<sys::vpx_codec_enc_cfg>::zeroed();
        unsafe {
            let iface = sys::vpx_codec_vp8_cx();
            let usage = 0; // ドキュメントでは、常に 0 を指定しろ、とのこと
            let code = sys::vpx_codec_enc_config_default(iface, cfg.as_mut_ptr(), usage);
            Error::check(code, "vpx_codec_enc_config_default")?;

            let cfg = cfg.assume_init();
            Self::new(config, cfg, iface)
        }
    }

    /// VP9 用のエンコーダーインスタンスを生成する
    pub fn new_vp9(config: &EncoderConfig) -> Result<Self, Error> {
        let mut cfg = MaybeUninit::<sys::vpx_codec_enc_cfg>::zeroed();
        unsafe {
            let iface = sys::vpx_codec_vp9_cx();
            let usage = 0; // ドキュメントでは、常に 0 を指定しろ、とのこと
            let code = sys::vpx_codec_enc_config_default(iface, cfg.as_mut_ptr(), usage);
            Error::check(code, "vpx_codec_enc_config_default")?;

            let cfg = cfg.assume_init();
            Self::new(config, cfg, iface)
        }
    }

    fn new(
        encoder_config: &EncoderConfig,
        mut vpx_config: sys::vpx_codec_enc_cfg,
        iface: *const sys::vpx_codec_iface,
    ) -> Result<Self, Error> {
        vpx_config.g_w = encoder_config.width as c_uint;
        vpx_config.g_h = encoder_config.height as c_uint;
        vpx_config.rc_target_bitrate = encoder_config.target_bitrate as c_uint / 1000;
        vpx_config.rc_min_quantizer = encoder_config.min_quantizer as c_uint;
        vpx_config.rc_max_quantizer = encoder_config.max_quantizer as c_uint;

        // FPS とは分子・分母の関係が逆になる
        vpx_config.g_timebase.num = encoder_config.fps_denominator as c_int;
        vpx_config.g_timebase.den = encoder_config.fps_numerator as c_int;

        let mut ctx = MaybeUninit::<sys::vpx_codec_ctx>::zeroed();
        unsafe {
            let code = sys::vpx_codec_enc_init_ver(
                ctx.as_mut_ptr(),
                iface,
                &vpx_config,
                0, // flags
                sys::VPX_ENCODER_ABI_VERSION as i32,
            );
            Error::check(code, "vpx_codec_enc_init_ver")?;

            let mut img = MaybeUninit::zeroed();
            sys::vpx_img_alloc(
                img.as_mut_ptr(),
                sys::vpx_img_fmt_VPX_IMG_FMT_I420,
                vpx_config.g_w,
                vpx_config.g_h,
                1, // align に 1 を指定することで width == y_stride となることが保証される
            );

            let img = img.assume_init();
            let mut this = Self {
                ctx: ctx.assume_init(),
                img,
                iter: std::ptr::null(),
                frame_count: 0,
                y_size: encoder_config.height * img.stride[0] as usize,
                u_size: encoder_config.height.div_ceil(2) * img.stride[1] as usize,
                v_size: encoder_config.height.div_ceil(2) * img.stride[2] as usize,
            };
            // NOTE: これ以降の操作に失敗しても ctx は Drop によって確実に解放される

            let code = sys::vpx_codec_control_(
                &mut this.ctx,
                // 名前に VP8 が含まれているけど、ドキュメントを見ると VP9 でも使える模様
                sys::vp8e_enc_control_id_VP8E_SET_CQ_LEVEL as c_int,
                encoder_config.cq_level as c_uint,
            );
            Error::check(code, "vpx_codec_control_")?;

            Ok(this)
        }
    }

    /// I420 形式の画像データをエンコードする
    ///
    /// エンコード結果は [`Encoder::next_frame()`] で取得できる
    ///
    /// なお `y` のストライドは入力フレームの幅と等しいことが前提
    pub fn encode(&mut self, y: &[u8], u: &[u8], v: &[u8]) -> Result<(), Error> {
        if !self.iter.is_null() {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_ERROR,
                "shiguredo_libvpx::Encoder::encode",
                "still need to call shiguredo_libvpx::Encoder::next_frame()",
            ));
        }
        if y.len() != self.y_size || u.len() != self.u_size || v.len() != self.v_size {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_INVALID_PARAM,
                "shiguredo_libvpx::Encoder::encode",
                "invalid YUV plane sizes",
            ));
        }

        let code = unsafe {
            std::slice::from_raw_parts_mut(self.img.planes[0], y.len()).copy_from_slice(y);
            std::slice::from_raw_parts_mut(self.img.planes[1], u.len()).copy_from_slice(u);
            std::slice::from_raw_parts_mut(self.img.planes[2], v.len()).copy_from_slice(v);

            sys::vpx_codec_encode(
                &mut self.ctx,
                &self.img,
                self.frame_count as sys::vpx_codec_pts_t,
                1, // duration: 1 は「1 フレーム分」を意味する
                0, // flags
                sys::VPX_DL_REALTIME as sys::vpx_enc_deadline_t,
            )
        };
        Error::check(code, "vpx_codec_encode")?;
        self.frame_count += 1;
        Ok(())
    }

    /// これ以上データが来ないことをエンコーダーに伝える
    ///
    /// 残りのエンコード結果は [`Encoder::next_frame()`] で取得できる
    pub fn finish(&mut self) -> Result<(), Error> {
        if !self.iter.is_null() {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_ERROR,
                "shiguredo_libvpx::Encoder::finish",
                "still need to call shiguredo_libvpx::Encoder::next_frame()",
            ));
        }

        let code = unsafe {
            sys::vpx_codec_encode(
                &mut self.ctx,
                std::ptr::null(),
                -1, // pts
                0,  // duration
                0,  // flags
                sys::VPX_DL_REALTIME as sys::vpx_enc_deadline_t,
            )
        };
        Error::check(code, "vpx_codec_encode")?;
        Ok(())
    }

    /// エンコード済みのフレームを取り出す
    ///
    /// [`Encoder::encode()`] や [`Encoder::finish()`] の後には、
    /// このメソッドを、結果が `None` になるまで呼び出し続ける必要がある
    pub fn next_frame(&mut self) -> Option<EncodedFrame> {
        unsafe {
            loop {
                let pkt = sys::vpx_codec_get_cx_data(&mut self.ctx, &mut self.iter);
                if pkt.is_null() {
                    self.iter = std::ptr::null();
                    break;
                }

                let pkt = &*pkt;
                if pkt.kind != sys::vpx_codec_cx_pkt_kind_VPX_CODEC_CX_FRAME_PKT {
                    continue;
                }

                return Some(EncodedFrame(&pkt.data.frame));
            }
        }
        None
    }
}

unsafe impl Send for Encoder {}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            sys::vpx_img_free(&mut self.img);
            sys::vpx_codec_destroy(&mut self.ctx);
        }
    }
}

impl std::fmt::Debug for Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encoder").finish_non_exhaustive()
    }
}

/// エンコードされた映像フレーム
pub struct EncodedFrame<'a>(&'a sys::vpx_codec_cx_pkt__bindgen_ty_1__bindgen_ty_1);

impl EncodedFrame<'_> {
    /// 圧縮データ
    pub fn data(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.0.buf as *mut u8, self.0.sz) }
    }

    /// フレームの幅
    pub fn width(&self) -> u16 {
        self.0.width[0] as u16
    }

    /// フレームの高さ
    pub fn height(&self) -> u16 {
        self.0.height[0] as u16
    }

    /// キーフレームかどうか
    pub fn is_keyframe(&self) -> bool {
        (self.0.flags & sys::VPX_FRAME_IS_KEY) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_vp8_decoder() {
        assert!(Decoder::new_vp8().is_ok());
    }

    #[test]
    fn init_vp9_decoder() {
        assert!(Decoder::new_vp9().is_ok());
    }

    #[test]
    fn decode_vp8_black() {
        let data = [
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
        let mut decoder = Decoder::new_vp8().expect("failed to create decoder");
        let mut decoded_count = 0;

        decoder.decode(&data).expect("failed to decode");
        while let Some(_) = decoder.next_frame() {
            decoded_count += 1;
        }

        decoder.finish().expect("failed to finish");
        while let Some(_) = decoder.next_frame() {
            decoded_count += 1;
        }

        assert_eq!(decoded_count, 1);
    }

    #[test]
    fn decode_vp9_black() {
        let data = [
            130, 73, 131, 66, 0, 39, 240, 29, 246, 0, 56, 36, 28, 24, 74, 16, 0, 80, 97, 246, 58,
            246, 128, 92, 209, 238, 0, 0, 0, 0, 0, 20, 103, 26, 154, 224, 98, 35, 126, 68, 120,
            240, 227, 199, 143, 30, 28, 238, 113, 218, 24, 0, 103, 26, 154, 224, 98, 35, 126, 68,
            120, 240, 227, 199, 143, 30, 28, 238, 113, 218, 24, 0,
        ];
        let mut decoder = Decoder::new_vp9().expect("failed to create decoder");
        let mut decoded_count = 0;

        decoder.decode(&data).expect("failed to decode");
        while let Some(_) = decoder.next_frame() {
            decoded_count += 1;
        }

        decoder.finish().expect("failed to finish");
        while let Some(_) = decoder.next_frame() {
            decoded_count += 1;
        }

        assert_eq!(decoded_count, 1);
    }

    #[test]
    fn init_vp8_encoder() {
        // OK
        let config = encoder_config();
        assert!(Encoder::new_vp8(&config).is_ok());

        // NG
        let mut config = encoder_config();
        config.fps_denominator = 0;
        assert!(Encoder::new_vp8(&config).is_err());
    }

    #[test]
    fn init_vp9_encoder() {
        // OK
        let config = encoder_config();
        assert!(Encoder::new_vp9(&config).is_ok());

        // NG
        let mut config = encoder_config();
        config.fps_denominator = 0;
        assert!(Encoder::new_vp9(&config).is_err());
    }

    #[test]
    fn encode_vp8_black() {
        let config = encoder_config();
        let mut encoder = Encoder::new_vp8(&config).expect("failed to create");
        let mut encoded_count = 0;

        let size = config.width * config.height;
        let y = vec![0; size];
        let u = vec![0; size / 4];
        let v = vec![0; size / 4];

        encoder.encode(&y, &u, &v).expect("failed to encode");
        while let Some(_) = encoder.next_frame() {
            encoded_count += 1;
        }

        encoder.finish().expect("failed to finish");
        while let Some(_) = encoder.next_frame() {
            encoded_count += 1;
        }

        assert_eq!(encoded_count, 1);
    }

    #[test]
    fn encode_vp9_black() {
        let config = encoder_config();
        let mut encoder = Encoder::new_vp9(&config).expect("failed to create");
        let mut encoded_count = 0;

        let size = config.width * config.height;
        let y = vec![0; size];
        let u = vec![0; size / 4];
        let v = vec![0; size / 4];

        encoder.encode(&y, &u, &v).expect("failed to encode");
        while let Some(_) = encoder.next_frame() {
            encoded_count += 1;
        }

        encoder.finish().expect("failed to finish");
        while let Some(_) = encoder.next_frame() {
            encoded_count += 1;
        }

        assert_eq!(encoded_count, 1);
    }

    fn encoder_config() -> EncoderConfig {
        EncoderConfig {
            width: 128,
            height: 128,
            fps_numerator: 30,
            fps_denominator: 1,
            target_bitrate: 1_000_000,
            min_quantizer: 1,
            max_quantizer: 1,
            cq_level: 1,
        }
    }

    #[test]
    fn error_reason() {
        let e = Error::check(sys::vpx_codec_err_t_VPX_CODEC_MEM_ERROR, "test")
            .expect_err("not an error");
        assert!(e.reason().is_some());
    }
}
