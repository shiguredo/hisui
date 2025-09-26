//! [Hisui] 用の [Audio Toolbox] デコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [Audio Toolbox]: https://developer.apple.com/documentation/audiotoolbox/
#![warn(missing_docs)]

use std::{ffi::c_void, mem::MaybeUninit};

mod sys;

/// エラー
#[derive(Debug)]
pub struct Error {
    status: i32,
    function: &'static str,
}

impl Error {
    fn check(status: i32, function: &'static str) -> Result<(), Self> {
        if status == 0 {
            return Ok(());
        }
        Err(Self { status, function })
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {}() failed: status={}",
            env!("CARGO_PKG_NAME"),
            self.function,
            self.status
        )
    }
}

impl std::error::Error for Error {}

// 以下のパラメータは Hisui では固定
const CHANNELS: usize = 2;
const SAMPLE_RATE: sys::Float64 = 48000.0;

// デコーダーコールバック内で使用する独自エラーコード。
// 理想的にはフレームワーク側と確実に衝突しない値を選択するべきだが、
// それを記載したドキュメントが見つからなかったので、実際に動かしてみて安全そうな値を使っている。
const K_NO_MORE_INPUT: i32 = 12345;

// エンコード結果を格納するためのバッファのサイズ
// 十分に大きなサイズならなんでもいい
const ENCODE_BUF_SIZE: usize = 4096;

/// AAC エンコーダー
#[derive(Debug)]
pub struct Encoder {
    converter: sys::AudioConverterRef,
    pcm_buf: Vec<i16>,
    eos: bool,
}

impl Encoder {
    /// エンコーダーインスタンスを生成する
    pub fn new(target_bitrate: usize) -> Result<Self, Error> {
        unsafe {
            let mut input_format =
                MaybeUninit::<sys::AudioStreamBasicDescription>::zeroed().assume_init();
            let mut output_format =
                MaybeUninit::<sys::AudioStreamBasicDescription>::zeroed().assume_init();

            input_format.mSampleRate = SAMPLE_RATE;
            input_format.mFormatID = sys::kAudioFormatLinearPCM;
            input_format.mFormatFlags =
                sys::kAudioFormatFlagIsSignedInteger | sys::kAudioFormatFlagIsPacked;
            input_format.mBytesPerPacket = 4;
            input_format.mFramesPerPacket = 1;
            input_format.mBytesPerFrame = 4;
            input_format.mChannelsPerFrame = CHANNELS as sys::UInt32;
            input_format.mBitsPerChannel = CHANNELS as sys::UInt32 * 8;

            // 以下の Table 2-6 が参考になる:
            // https://developer.apple.com/library/archive/documentation/MusicAudio/Reference/CAFSpec/CAF_spec/CAF_spec.html
            output_format.mSampleRate = SAMPLE_RATE;
            output_format.mFormatID = sys::kAudioFormatMPEG4AAC;
            output_format.mFormatFlags = sys::kMPEG4Object_AAC_LC;
            output_format.mChannelsPerFrame = CHANNELS as sys::UInt32;
            output_format.mBitsPerChannel = 0;
            output_format.mFramesPerPacket = 1024;
            output_format.mBytesPerPacket = 0;

            let mut converter = std::ptr::null_mut();
            let status = sys::AudioConverterNew(&input_format, &output_format, &mut converter);
            Error::check(status, "AudioConverterNew")?;

            // ビットレート指定
            let status = sys::AudioConverterSetProperty(
                converter,
                sys::kAudioConverterEncodeBitRate,
                size_of::<u32>() as sys::UInt32,
                (&(target_bitrate as u32) as *const u32).cast(),
            );
            Error::check(status, "AudioConverterSetProperty")?;

            Ok(Self {
                converter,
                pcm_buf: Vec::new(),
                eos: false,
            })
        }
    }

    /// PCM 音声データをエンコードする
    pub fn encode(&mut self, pcm: &[i16]) -> Result<Option<EncodedFrame>, Error> {
        self.pcm_buf.extend_from_slice(pcm);
        self.encode_impl()
    }

    /// エンコーダーに、これ以上データが来ないこと、を伝えて残りのエンコード結果を取得する
    pub fn finish(&mut self) -> Result<Option<EncodedFrame>, Error> {
        self.eos = true;
        self.encode_impl()
    }

    fn encode_impl(&mut self) -> Result<Option<EncodedFrame>, Error> {
        let mut encoded_data = [0; ENCODE_BUF_SIZE];
        let mut io_packets = 1;
        let mut output_buffer_list =
            unsafe { MaybeUninit::<sys::AudioBufferList>::zeroed().assume_init() };
        output_buffer_list.mNumberBuffers = 1;
        output_buffer_list.mBuffers[0].mNumberChannels = CHANNELS as sys::UInt32;
        output_buffer_list.mBuffers[0].mData = encoded_data.as_mut_ptr().cast();
        output_buffer_list.mBuffers[0].mDataByteSize = encoded_data.len() as u32;

        let old_samples = self.pcm_buf.len() / CHANNELS;
        let status = unsafe {
            sys::AudioConverterFillComplexBuffer(
                self.converter,
                Some(Self::callback),
                (self as *mut Self).cast(),
                &mut io_packets,
                &mut output_buffer_list,
                std::ptr::null_mut(),
            )
        };
        if status == K_NO_MORE_INPUT {
            return Ok(None);
        }
        Error::check(status, "AudioConverterFillComplexBuffer")?;

        let size = output_buffer_list.mBuffers[0].mDataByteSize as usize;
        Ok(Some(EncodedFrame {
            data: encoded_data[..size].to_vec(),
            samples: old_samples - (self.pcm_buf.len() / CHANNELS),
        }))
    }

    unsafe extern "C" fn callback(
        _in_audio_converter: sys::AudioConverterRef,
        io_number_data_packets: *mut u32,
        io_data: *mut sys::AudioBufferList,
        _out_data_packet_description: *mut *mut sys::AudioStreamPacketDescription,
        in_user_data: *mut c_void,
    ) -> i32 {
        // [NOTE] Video Toolbox とは異なり、Audio ではコールバックが同じスレッド内で実行される
        unsafe {
            let this: &mut Encoder = &mut *(in_user_data as *mut Encoder);
            let packets = *io_number_data_packets;
            if !this.eos && this.pcm_buf.len() < packets as usize * CHANNELS {
                return K_NO_MORE_INPUT;
            }

            *io_number_data_packets = packets.min((this.pcm_buf.len() / CHANNELS) as u32);

            let packets = *io_number_data_packets;
            let io_data = &mut *io_data;
            let size = packets * CHANNELS as u32 * size_of::<i16>() as u32;
            io_data.mNumberBuffers = 1;
            io_data.mBuffers[0].mNumberChannels = 2;
            std::slice::from_raw_parts_mut(
                io_data.mBuffers[0].mData.cast(),
                size as usize / CHANNELS,
            )
            .copy_from_slice(&this.pcm_buf[..size as usize / CHANNELS]);

            io_data.mBuffers[0].mDataByteSize = size;
            this.pcm_buf.drain(0..packets as usize * CHANNELS);
        }
        sys::noErr as i32
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            sys::AudioConverterDispose(self.converter);
        }
    }
}

unsafe impl Send for Encoder {}

/// エンコードされた音声フレーム
#[derive(Debug)]
pub struct EncodedFrame {
    /// 圧縮データ
    pub data: Vec<u8>,

    /// フレームに含まれているサンプルの数
    pub samples: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_encoder() {
        // OK
        assert!(Encoder::new(128_000).is_ok());

        // NG
        assert!(Encoder::new(1_000).is_err());
    }

    #[test]
    fn encode_silent() {
        let mut encoder = Encoder::new(128_000).expect("create encoder error");
        let mut sample_count = 0;

        for _ in 0..100 {
            if let Some(encoded) = encoder.encode(&[0; 100 * CHANNELS]).expect("encode error") {
                sample_count += encoded.samples;
            }
        }
        if let Some(encoded) = encoder.finish().expect("finish error") {
            sample_count += encoded.samples;
        }

        assert_eq!(sample_count, 100 * 100);
    }
}
