use shiguredo_mp4::boxes::{DopsBox, OpusBox, SampleEntry};

use crate::audio::{self, Channels, SampleRate};

/// Opus 用の sample_entry を構築する。
///
/// 引数 `pre_skip` は OpusHead (RFC 7845 §5.1) から取得した値。
/// channels / sample_rate / output_gain は Hisui の固定値 (Stereo / 48kHz / 0) を使う。
pub fn opus_sample_entry(pre_skip: u16) -> SampleEntry {
    SampleEntry::Opus(OpusBox {
        audio: audio::sample_entry_audio_fields(),
        dops_box: DopsBox {
            output_channel_count: Channels::STEREO.get(),
            pre_skip,
            input_sample_rate: SampleRate::HZ_48000.get(),
            output_gain: 0,
        },
        unknown_boxes: Vec::new(),
    })
}
