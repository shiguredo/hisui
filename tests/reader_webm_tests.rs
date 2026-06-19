use hisui::sample_entry::SharedSampleEntry;
use hisui::webm::reader::{WebmAudioReader, WebmVideoReader};
use shiguredo_mp4::boxes::SampleEntry;

// testdata/archive-black-silent.webm の実値 (mkvinfo + xxd で実測)。
// Sora 録画では lookahead が無いため OpusHead の pre_skip は 0。
const TESTDATA_OPUS_PRE_SKIP: u16 = 0;
const TESTDATA_VP8_WIDTH: u16 = 640;
const TESTDATA_VP8_HEIGHT: u16 = 480;

#[test]
fn webm_audio_reader_test() -> hisui::Result<()> {
    let mut reader = WebmAudioReader::new("testdata/archive-black-silent.webm")?;
    let mut last_timestamp = None;
    let mut first_sample_entry: Option<SharedSampleEntry> = None;
    for audio_data in reader.by_ref() {
        let audio_data = audio_data?;
        last_timestamp = Some(audio_data.timestamp);

        // 不変条件 (圧縮 AudioFrame は常に Some) は src/audio.rs::AudioFrame の docstring を参照。
        let sample_entry = audio_data
            .sample_entry
            .expect("WebmAudioReader が返す Opus フレームには常に sample_entry が載る");

        // ファイル単位で 1 つの SharedSampleEntry を共有するため、初回と後続は同一 Arc になる。
        match &first_sample_entry {
            None => {
                // 初回フレームでは中身を実 testdata の OpusHead 由来の pre_skip と照合する。
                // ここが壊れていると OpusHead パースの退行を検出できなくなる。
                match sample_entry.get() {
                    SampleEntry::Opus(opus_box) => assert_eq!(
                        opus_box.dops_box.pre_skip, TESTDATA_OPUS_PRE_SKIP,
                        "sample_entry の pre_skip が testdata の OpusHead と一致すること"
                    ),
                    other => panic!("Opus 経路で SampleEntry::Opus 以外が返った: {other:?}"),
                }
                first_sample_entry = Some(sample_entry);
            }
            Some(first) => assert!(
                first.ptr_eq(&sample_entry),
                "後続フレームの sample_entry は初回フレームと同一 Arc を共有すること"
            ),
        }
    }
    if let Some(last_timestamp) = last_timestamp {
        assert_eq!(reader.stats().total_track_duration, last_timestamp);
    }
    Ok(())
}

#[test]
fn webm_video_reader_test() -> hisui::Result<()> {
    let mut reader = WebmVideoReader::new("testdata/archive-black-silent.webm")?;
    let mut last_timestamp = None;
    let mut first_sample_entry: Option<SharedSampleEntry> = None;
    for video_frame in reader.by_ref() {
        let video_frame = video_frame?;
        last_timestamp = Some(video_frame.timestamp);

        // 不変条件 (圧縮 VideoFrame は常に Some) は src/video.rs::VideoFrame の docstring を参照。
        let sample_entry = video_frame
            .sample_entry
            .expect("WebmVideoReader が返す VP8 フレームには常に sample_entry が載る");

        // ファイル単位で 1 つの SharedSampleEntry を共有するため、初回と後続は同一 Arc になる。
        match &first_sample_entry {
            None => {
                // 初回フレームでは中身を実 testdata の PixelWidth / PixelHeight と照合する。
                // ここが壊れていると VideoTrackHeader の Pixel 寸法解析の退行を検出できなくなる。
                match sample_entry.get() {
                    SampleEntry::Vp08(vp08_box) => {
                        assert_eq!(
                            vp08_box.visual.width, TESTDATA_VP8_WIDTH,
                            "sample_entry の width が testdata の PixelWidth と一致すること"
                        );
                        assert_eq!(
                            vp08_box.visual.height, TESTDATA_VP8_HEIGHT,
                            "sample_entry の height が testdata の PixelHeight と一致すること"
                        );
                    }
                    other => panic!("VP8 経路で SampleEntry::Vp08 以外が返った: {other:?}"),
                }
                first_sample_entry = Some(sample_entry);
            }
            Some(first) => assert!(
                first.ptr_eq(&sample_entry),
                "後続フレームの sample_entry は初回フレームと同一 Arc を共有すること"
            ),
        }
    }
    if let Some(last_timestamp) = last_timestamp {
        assert_eq!(reader.stats().total_track_duration, last_timestamp);
    }
    Ok(())
}
