use crate::audio::{Channels, SampleRate};

use shiguredo_mp4::{
    FixedPointNumber, Uint,
    boxes::{AudioSampleEntryFields, EsdsBox, Mp4aBox, SampleEntry},
    descriptors::{DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor},
};

pub fn parse_audio_specific_config(data: &[u8]) -> crate::Result<(SampleRate, Channels)> {
    if data.len() < 2 {
        return Err(crate::Error::new("AAC audio specific config is too short"));
    }

    let byte0 = data[0];
    let byte1 = data[1];
    let sample_rate_index = ((byte0 & 0x07) << 1) | (byte1 >> 7);
    let channel_configuration = (byte1 >> 3) & 0x0F;

    let sample_rate = sample_rate_from_sampling_frequency_index(sample_rate_index)?;
    let channels = Channels::from_u8(channel_configuration)?;
    Ok((sample_rate, channels))
}

pub fn sample_rate_from_sampling_frequency_index(index: u8) -> crate::Result<SampleRate> {
    let sample_rate = match index {
        0 => 96_000,
        1 => 88_200,
        2 => 64_000,
        3 => 48_000,
        4 => 44_100,
        5 => 32_000,
        6 => 24_000,
        7 => 22_050,
        8 => 16_000,
        9 => 12_000,
        10 => 11_025,
        11 => 8_000,
        12 => 7_350,
        _ => return Err(crate::Error::new("invalid AAC sample rate index")),
    };
    SampleRate::from_u32(sample_rate)
}

pub fn create_audio_specific_config(
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u8,
) -> Vec<u8> {
    let byte0 = (audio_object_type << 3) | ((sampling_frequency_index >> 1) & 0x07);
    let byte1 = ((sampling_frequency_index & 0x01) << 7) | ((channel_configuration & 0x0F) << 3);
    vec![byte0, byte1]
}

pub fn create_mp4a_sample_entry(
    audio_specific_config: &[u8],
    sample_rate: SampleRate,
    channels: Channels,
) -> crate::Result<SampleEntry> {
    let sample_rate_u16 = sample_rate.as_u16()?;

    Ok(SampleEntry::Mp4a(Mp4aBox {
        audio: AudioSampleEntryFields {
            data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            channelcount: u16::from(channels.get()),
            samplesize: 16,
            samplerate: FixedPointNumber::new(sample_rate_u16, 0),
        },
        esds_box: EsdsBox {
            es: EsDescriptor {
                es_id: EsDescriptor::MIN_ES_ID,
                stream_priority: EsDescriptor::LOWEST_STREAM_PRIORITY,
                depends_on_es_id: None,
                url_string: None,
                ocr_es_id: None,
                dec_config_descr: DecoderConfigDescriptor {
                    object_type_indication:
                        DecoderConfigDescriptor::OBJECT_TYPE_INDICATION_AUDIO_ISO_IEC_14496_3,
                    stream_type: DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                    up_stream: DecoderConfigDescriptor::UP_STREAM_FALSE,
                    dec_specific_info: Some(DecoderSpecificInfo {
                        payload: audio_specific_config.to_vec(),
                    }),
                    buffer_size_db: Uint::new(65536),
                    max_bitrate: 256000,
                    avg_bitrate: 128000,
                },
                sl_config_descr: shiguredo_mp4::descriptors::SlConfigDescriptor,
            },
        },
        unknown_boxes: Vec::new(),
    }))
}

/// MPEG4-GENERIC mode AAC RTP の 1 AU 取り出し時間 (= 1024 サンプル単位、ISO/IEC 14496-3)。
const AAC_AU_SAMPLES: u64 = 1024;

/// MPEG4-GENERIC AAC RTP の AU header と AU データを取り出すデパケッタイザー。
///
/// RFC 3640 §3.3.6 で規定される `sizeLength` / `indexLength` / `indexDeltaLength` の
/// fmtp パラメータを受けて、RTP payload から複数の AU を取り出す。
///
/// 現実装は AU-Index / AU-Index-delta を読み捨て、各 AU の RTP タイムスタンプを packet
/// header の timestamp と Vec 内位置から計算する。RFC 3640 §3.2.1 の interleaving モード
/// (publisher が AU を並び替えて送信し、受信側が AU-Index で並べ直す経路) は非対応。
/// Sora 等の典型 publisher は non-interleaved (AU-Index = 0, AU-Index-delta = 0) で
/// 送信するため実害はない。
#[derive(Debug)]
pub struct AacRtpDepacketizer {
    size_length: u8,
    index_length: u8,
    index_delta_length: u8,
}

/// RTP payload から取り出した 1 AU の情報。
#[derive(Debug, Clone)]
pub struct AudioAccessUnit {
    /// AU 単位で再計算した RTP タイムスタンプ。
    pub rtp_timestamp: u32,
    /// AU バイト列 (生 AAC データ)。
    pub data: Vec<u8>,
}

impl AacRtpDepacketizer {
    pub fn new(size_length: u8, index_length: u8, index_delta_length: u8) -> Self {
        // size_length が 0 だと depacketize 内の AU header ループで consumed_bits が
        // 進まず無限ループに陥る。本番経路では validate_aac_fmtp_lengths で弾かれるが、
        // struct の不変条件として debug ビルドで明示的に固定する。
        debug_assert!(
            size_length > 0,
            "AacRtpDepacketizer requires size_length > 0"
        );
        Self {
            size_length,
            index_length,
            index_delta_length,
        }
    }

    pub fn depacketize(
        &self,
        packet: &shiguredo_rtsp::RtpPacket,
    ) -> crate::Result<Vec<AudioAccessUnit>> {
        if packet.payload.len() < 2 {
            return Err(crate::Error::new(
                "invalid AAC RTP payload: missing AU header length",
            ));
        }

        let au_headers_length_bits =
            u16::from_be_bytes([packet.payload[0], packet.payload[1]]) as usize;
        if au_headers_length_bits == 0 {
            return Err(crate::Error::new(
                "invalid AAC RTP payload: AU header length must be greater than 0",
            ));
        }
        let au_headers_length_bytes = au_headers_length_bits.div_ceil(8);
        if packet.payload.len() < 2 + au_headers_length_bytes {
            return Err(crate::Error::new(
                "invalid AAC RTP payload: AU headers are truncated",
            ));
        }

        // 共有 BitReader の Err は AU header 経路と SPS パース経路で同一文言になるため、
        // ログから経路を識別できるように with_context で AU header 由来を前置する。
        const AAC_AU_HEADER_CONTEXT: &str = "invalid AAC AU header";

        let au_headers = &packet.payload[2..2 + au_headers_length_bytes];
        let mut bit_reader = crate::video::bit_reader::BitReader::new(au_headers);
        let mut au_sizes = Vec::new();
        let mut first = true;
        // size_length / index_bits はそれぞれ 32 以下に検査済みで、au_headers_length_bits は
        // u16 由来の usize 値のため、consumed_bits の overflow は発生しない。
        //
        // 現実装は AU-Index / AU-Index-delta を読み捨て、各 AU の RTP タイムスタンプを
        // packet header の timestamp と Vec 内位置から計算する (後段の data_offset ループ参照)。
        // RFC 3640 §3.2.1 の interleaving モード (publisher が AU を並び替えて送信し、
        // 受信側が AU-Index で並べ直す経路) は非対応。Sora 等の典型 publisher は
        // non-interleaved (AU-Index = 0, AU-Index-delta = 0) で送信するため実害はない。
        let mut consumed_bits = 0usize;
        while consumed_bits < au_headers_length_bits {
            let size = bit_reader
                .read_u(self.size_length as usize)
                .map_err(|e| e.with_context(AAC_AU_HEADER_CONTEXT))?
                as usize;
            consumed_bits += self.size_length as usize;
            let index_bits = if first {
                self.index_length
            } else {
                self.index_delta_length
            };
            let _ = bit_reader
                .read_u(index_bits as usize)
                .map_err(|e| e.with_context(AAC_AU_HEADER_CONTEXT))?;
            consumed_bits += index_bits as usize;
            first = false;
            au_sizes.push(size);
        }

        let mut data_offset = 2 + au_headers_length_bytes;
        let mut access_units = Vec::with_capacity(au_sizes.len());
        for (index, au_size) in au_sizes.into_iter().enumerate() {
            if data_offset + au_size > packet.payload.len() {
                return Err(crate::Error::new(
                    "invalid AAC RTP payload: AU data is truncated",
                ));
            }

            let raw_timestamp = packet
                .header
                .timestamp
                .wrapping_add((index as u32).saturating_mul(AAC_AU_SAMPLES as u32));
            access_units.push(AudioAccessUnit {
                rtp_timestamp: raw_timestamp,
                data: packet.payload[data_offset..data_offset + au_size].to_vec(),
            });
            data_offset += au_size;
        }

        Ok(access_units)
    }
}

/// RFC 3640 §3.3.6 の `sizeLength` / `indexLength` / `indexDeltaLength` の値域を検査する。
///
/// `sizeLength == 0` および `> 32` を Err 化する。32 超を弾く根拠は共有 `BitReader::read_u`
/// の制約 (n > 32 で Err) で、SDP fmtp 受領時点 (RTSP の `select_audio_track`) と PBT の
/// 双方から呼ぶ単一情報源。
pub fn validate_aac_fmtp_lengths(
    size_length: u8,
    index_length: u8,
    index_delta_length: u8,
) -> crate::Result<()> {
    if size_length == 0 {
        return Err(crate::Error::new(
            "AAC fmtp sizeLength must be greater than 0",
        ));
    }
    if size_length > 32 {
        return Err(crate::Error::new("AAC fmtp sizeLength must be 32 or less"));
    }
    if index_length > 32 {
        return Err(crate::Error::new("AAC fmtp indexLength must be 32 or less"));
    }
    if index_delta_length > 32 {
        return Err(crate::Error::new(
            "AAC fmtp indexDeltaLength must be 32 or less",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_audio_specific_config_parses_basic_values() {
        let (sample_rate, channels) =
            parse_audio_specific_config(&[0x12, 0x10]).expect("must parse config");
        assert_eq!(sample_rate.get(), 44_100);
        assert_eq!(channels.get(), 2);
    }

    #[test]
    fn create_mp4a_sample_entry_keeps_audio_specific_config() {
        let sample_entry = create_mp4a_sample_entry(
            &[0x12, 0x10],
            SampleRate::from_u32(44_100).expect("must create sample rate"),
            Channels::STEREO,
        )
        .expect("must create sample entry");

        let SampleEntry::Mp4a(mp4a) = sample_entry else {
            panic!("expected Mp4a sample entry");
        };

        assert_eq!(mp4a.audio.channelcount, 2);
        assert_eq!(mp4a.audio.samplerate.integer, 44_100);
        assert_eq!(
            mp4a.esds_box
                .es
                .dec_config_descr
                .dec_specific_info
                .as_ref()
                .expect("AudioSpecificConfig must exist")
                .payload,
            vec![0x12, 0x10]
        );
    }
}
