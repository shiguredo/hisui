//! [Hisui] 用の [dav1d] デコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [dav1d]: https://github.com/videolan/dav1d
#![warn(missing_docs)]

use std::{ffi::c_int, mem::MaybeUninit};

mod sys;

/// エラー
#[derive(Debug)]
pub struct Error {
    code: c_int,
    function: &'static str,
}

impl Error {
    fn check(code: c_int, function: &'static str) -> Result<(), Self> {
        if code == 0 {
            Ok(())
        } else {
            Err(Self { code, function })
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}() failed: code={}", self.function, self.code)
    }
}

impl std::error::Error for Error {}

/// AV1 デコーダー
#[derive(Debug)]
pub struct Decoder {
    ctx: *mut sys::Dav1dContext,
}

impl Decoder {
    /// AV1 デコーダーインスタンスを生成する
    pub fn new() -> Result<Self, Error> {
        let mut settings = MaybeUninit::<sys::Dav1dSettings>::zeroed();
        unsafe {
            sys::dav1d_default_settings(settings.as_mut_ptr());

            let settings = settings.assume_init();
            let mut ctx = std::ptr::null_mut();
            let code = sys::dav1d_open(&mut ctx, &settings);
            Error::check(code, "dav1d_open")?;

            Ok(Self { ctx })
        }
    }

    /// 圧縮された映像フレームをデコードする
    ///
    /// デコード結果は [`Decoder::next_frame()`] で取得できる
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        let mut dav1d_data = MaybeUninit::<sys::Dav1dData>::zeroed();
        unsafe {
            let dav1d_data_buf_ptr = sys::dav1d_data_create(dav1d_data.as_mut_ptr(), data.len());
            if dav1d_data_buf_ptr.is_null() {
                // dav1d の慣習に倣ってエラーコードは負数にする
                Error::check(-(sys::ENOMEM as c_int), "dav1d_data_create")?;
            }
            std::slice::from_raw_parts_mut(dav1d_data_buf_ptr, data.len()).copy_from_slice(data);

            let mut dav1d_data = dav1d_data.assume_init();
            let code = sys::dav1d_send_data(self.ctx, &mut dav1d_data);
            Error::check(code, "dav1d_send_data").inspect_err(|_| {
                sys::dav1d_data_unref(&mut dav1d_data);
            })?;
        }
        Ok(())
    }

    /// これ以上データが来ないことをデコーダーに伝える
    ///
    /// 残りのデコード結果は [`Decoder::next_frame()`] で取得できる
    pub fn finish(&mut self) -> Result<(), Error> {
        // [NOTE]
        // dav1d では dav1d_get_picture() が EAGAIN を返した後にもう一度
        // 同じ関数を呼び出すと、強制的にバッファ内のデコード画像取得されるようになる。
        // そのため、finish() の中で特にやることはないが、他のライブラリのデコーダのインタフェースに
        // 合わせておいた方が分かりやすいので、メソッドの枠だけは用意している。
        Ok(())
    }

    /// デコード済みのフレームを取り出す
    ///
    /// [`Decoder::decode()`] や [`Decoder::finish()`] の後には、
    /// このメソッドを、結果が `None` になるまで呼び出し続ける必要がある
    pub fn next_frame(&mut self) -> Result<Option<DecodedFrame>, Error> {
        let mut picture = MaybeUninit::<sys::Dav1dPicture>::zeroed();
        unsafe {
            let code = sys::dav1d_get_picture(self.ctx, picture.as_mut_ptr());
            if code < 0 && code.unsigned_abs() == sys::EAGAIN {
                return Ok(None);
            }
            Error::check(code, "dav1d_get_picture")?;

            let picture = picture.assume_init();

            // I420 前提
            assert_eq!(
                picture.p.layout,
                sys::Dav1dPixelLayout_DAV1D_PIXEL_LAYOUT_I420
            );

            Ok(Some(DecodedFrame(picture)))
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            sys::dav1d_close(&mut self.ctx);
        }
    }
}

unsafe impl Send for Decoder {}

/// デコードされた映像フレーム (I420 形式)
#[derive(Debug)]
pub struct DecodedFrame(sys::Dav1dPicture);

impl DecodedFrame {
    /// フレームの Y 成分のデータを返す
    pub fn y_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.0.data[0].cast_const().cast(),
                self.height() * self.y_stride(),
            )
        }
    }

    /// フレームの U 成分のデータを返す
    pub fn u_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.0.data[1].cast_const().cast(),
                self.height().div_ceil(2) * self.u_stride(),
            )
        }
    }

    /// フレームの V 成分のデータを返す
    pub fn v_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.0.data[2].cast_const().cast(),
                self.height().div_ceil(2) * self.v_stride(),
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
        self.u_stride() // U と V は共通
    }

    /// フレームの幅を返す
    pub fn width(&self) -> usize {
        self.0.p.w as usize
    }

    /// フレームの高さを返す
    pub fn height(&self) -> usize {
        self.0.p.h as usize
    }
}

impl Drop for DecodedFrame {
    fn drop(&mut self) {
        unsafe {
            sys::dav1d_picture_unref(&mut self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_decoder() {
        assert!(Decoder::new().is_ok());
    }

    #[test]
    fn decode_black() {
        let data = [
            10, 11, 0, 0, 0, 36, 196, 255, 223, 63, 254, 96, 16, 50, 35, 16, 0, 144, 0, 0, 0, 160,
            0, 0, 128, 1, 197, 120, 80, 103, 179, 239, 241, 100, 76, 173, 116, 93, 183, 31, 101,
            221, 87, 90, 233, 219, 28, 199, 243, 128,
        ];
        let mut decoder = Decoder::new().expect("new() error");
        let mut count = 0;

        decoder.decode(&data).expect("decode() error");
        while let Ok(Some(_)) = decoder.next_frame() {
            count += 1;
        }

        decoder.finish().expect("finish() error");
        while let Ok(Some(_)) = decoder.next_frame() {
            count += 1;
        }

        assert_eq!(count, 1);
    }
}
