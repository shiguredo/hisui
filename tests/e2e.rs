fn run_hisui_command(args: &[&str]) -> noargs::Result<std::process::Output> {
    let hisui_bin = env!("CARGO_BIN_EXE_hisui");
    let output = std::process::Command::new(hisui_bin)
        .args(["--verbose"])
        .args(args)
        .output()?;

    eprintln!("hisui args: --verbose {}", args.join(" "));
    eprintln!("hisui stdout:\n{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("hisui stderr:\n{}", String::from_utf8_lossy(&output.stderr));

    if !output.status.success() {
        return Err("hisui command failed".into());
    }

    Ok(output)
}

#[test]
fn inspect_mp4_without_decode() -> noargs::Result<()> {
    let output = run_hisui_command(&["inspect", "testdata/archive-red-320x320-vp9.mp4"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("inspect 出力の JSON パースに失敗: {e}"))?;

    let root = json.value();
    assert_eq!(
        root.to_member("format")?
            .required()?
            .to_unquoted_string_str()?,
        "mp4"
    );

    let mut video_sample_count = 0;
    let mut has_decoded_data_size = false;
    for sample in root.to_member("video_samples")?.required()?.to_array()? {
        video_sample_count += 1;
        if sample.to_member("decoded_data_size")?.optional().is_some() {
            has_decoded_data_size = true;
        }
    }

    assert!(video_sample_count > 0, "video sample must exist");
    assert!(
        !has_decoded_data_size,
        "decoded_data_size must not exist without --decode",
    );
    Ok(())
}

#[test]
fn inspect_fragmented_mp4_video_only() -> noargs::Result<()> {
    let plain_stdout = inspect_stdout("testdata/archive-red-320x320-h264.mp4")?;
    let fmp4_stdout = inspect_stdout("testdata/archive-red-320x320-h264-fragmented.mp4")?;

    assert_inspect_format_and_codec(&plain_stdout, "mp4", None, Some("H264"))?;
    assert_inspect_format_and_codec(&fmp4_stdout, "fmp4", None, Some("H264"))?;

    let plain = extract_inspect_comparable_samples(&plain_stdout)?;
    let fmp4 = extract_inspect_comparable_samples(&fmp4_stdout)?;

    let plain_video = plain
        .video
        .expect("testdata/archive-red-320x320-h264.mp4 に video_samples が存在すること");
    let fmp4_video = fmp4
        .video
        .expect("testdata/archive-red-320x320-h264-fragmented.mp4 に video_samples が存在すること");
    assert_eq!(
        fmp4_video, plain_video,
        "映像サンプルの data_size / keyframe / nalus が通常 MP4 と fMP4 で一致すること"
    );

    // testdata 再生成時に通常 MP4 と fMP4 が同時に同数で変化した場合の回帰を検出するため、
    // 絶対値もアンカーとして固定する。
    assert_eq!(fmp4_video.len(), 25, "映像サンプル数 25 (回帰検出アンカー)");

    Ok(())
}

#[test]
fn inspect_fragmented_mp4_audio_only() -> noargs::Result<()> {
    let plain_stdout = inspect_stdout("testdata/beep-aac-audio.mp4")?;
    let fmp4_stdout = inspect_stdout("testdata/beep-aac-audio-fragmented.mp4")?;

    assert_inspect_format_and_codec(&plain_stdout, "mp4", Some("AAC"), None)?;
    assert_inspect_format_and_codec(&fmp4_stdout, "fmp4", Some("AAC"), None)?;

    let plain = extract_inspect_comparable_samples(&plain_stdout)?;
    let fmp4 = extract_inspect_comparable_samples(&fmp4_stdout)?;

    let plain_audio = plain
        .audio
        .expect("testdata/beep-aac-audio.mp4 に audio_samples が存在すること");
    let fmp4_audio = fmp4
        .audio
        .expect("testdata/beep-aac-audio-fragmented.mp4 に audio_samples が存在すること");
    assert_eq!(
        fmp4_audio, plain_audio,
        "音声サンプルの data_size が通常 MP4 と fMP4 で一致すること"
    );

    assert_eq!(fmp4_audio.len(), 45, "音声サンプル数 45 (回帰検出アンカー)");

    Ok(())
}

#[test]
fn inspect_fragmented_mp4_audio_video() -> noargs::Result<()> {
    let plain_stdout = inspect_stdout("testdata/red-320x320-h264-aac.mp4")?;
    let fmp4_stdout = inspect_stdout("testdata/red-320x320-h264-aac-fragmented.mp4")?;

    assert_inspect_format_and_codec(&plain_stdout, "mp4", Some("AAC"), Some("H264"))?;
    assert_inspect_format_and_codec(&fmp4_stdout, "fmp4", Some("AAC"), Some("H264"))?;

    let plain = extract_inspect_comparable_samples(&plain_stdout)?;
    let fmp4 = extract_inspect_comparable_samples(&fmp4_stdout)?;

    let plain_audio = plain
        .audio
        .expect("testdata/red-320x320-h264-aac.mp4 に audio_samples が存在すること");
    let fmp4_audio = fmp4
        .audio
        .expect("testdata/red-320x320-h264-aac-fragmented.mp4 に audio_samples が存在すること");
    let plain_video = plain
        .video
        .expect("testdata/red-320x320-h264-aac.mp4 に video_samples が存在すること");
    let fmp4_video = fmp4
        .video
        .expect("testdata/red-320x320-h264-aac-fragmented.mp4 に video_samples が存在すること");

    assert_eq!(
        fmp4_audio, plain_audio,
        "音声サンプルの data_size が通常 MP4 と fMP4 で一致すること"
    );
    assert_eq!(
        fmp4_video, plain_video,
        "映像サンプルの data_size / keyframe / nalus が通常 MP4 と fMP4 で一致すること"
    );

    assert_eq!(fmp4_audio.len(), 45, "音声サンプル数 45 (回帰検出アンカー)");
    assert_eq!(fmp4_video.len(), 25, "映像サンプル数 25 (回帰検出アンカー)");

    Ok(())
}

#[test]
fn inspect_mp4_with_decode() -> noargs::Result<()> {
    let output = run_hisui_command(&[
        "inspect",
        "--decode",
        "testdata/archive-red-320x320-vp9.mp4",
    ])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("inspect 出力の JSON パースに失敗: {e}"))?;

    let root = json.value();
    assert_eq!(
        root.to_member("format")?
            .required()?
            .to_unquoted_string_str()?,
        "mp4"
    );

    let mut video_sample_count = 0;
    let mut has_decoded_data_size = false;
    let mut has_resolution = false;
    for sample in root.to_member("video_samples")?.required()?.to_array()? {
        video_sample_count += 1;
        if sample.to_member("decoded_data_size")?.optional().is_some() {
            has_decoded_data_size = true;
        }
        let has_width = sample.to_member("width")?.optional().is_some();
        let has_height = sample.to_member("height")?.optional().is_some();
        if has_width && has_height {
            has_resolution = true;
        }
    }

    assert!(video_sample_count > 0, "video sample must exist");
    assert!(
        has_decoded_data_size,
        "decoded_data_size must exist with --decode",
    );
    assert!(has_resolution, "width and height must exist with --decode");
    Ok(())
}

/// 単一 stsd + ビットストリーム内パラメータセット変化 (ffmpeg concat 出力) の H.264 入力で、
/// 全サンプルが VideoToolbox でデコードできることを確認する回帰テスト。
///
/// 修正前は後半の解像度変更で `status=-12909` が発生し、後半サンプルの
/// `decoded_data_size` / `width` / `height` が欠落していた。
#[test]
#[cfg(target_os = "macos")]
fn inspect_mp4_with_decode_h264_resolution_change() -> noargs::Result<()> {
    assert_resolution_change_inspect_ok("testdata/h264-resolution-change.mp4", "H264")
}

/// 単一 stsd + in-band パラメータセット変化 (hev1) の H.265 入力で、
/// 全サンプルが VideoToolbox でデコードできることを確認する回帰テスト。
///
/// 修正前は後半の解像度変更で `status=-12909` が発生し、後半サンプルの
/// `decoded_data_size` / `width` / `height` が欠落していた。
#[test]
#[cfg(target_os = "macos")]
fn inspect_mp4_with_decode_h265_resolution_change() -> noargs::Result<()> {
    assert_resolution_change_inspect_ok("testdata/h265-resolution-change.mp4", "H265")
}

/// 単一 stsd + ビットストリーム内パラメータセット変化の MP4 を VideoToolbox で
/// デコードし、全サンプルに `decoded_data_size` / `width` / `height` が付くことと、
/// 後半 25 サンプルが 320x320 でデコードされることを確認する共通ヘルパー。
///
/// `assert_inspect_format_and_codec` で codec をピン留めし、テストデータの取り違え
/// (H.264 用テストに H.265 データを渡す等) を早期に検出する。
fn assert_resolution_change_inspect_ok(path: &str, expected_codec: &str) -> noargs::Result<()> {
    let output = run_hisui_command(&["inspect", "--decode", path])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("status=-12909"),
        "VideoToolbox デコードエラー (status=-12909) が発生しないこと"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_inspect_format_and_codec(&stdout, "mp4", None, Some(expected_codec))?;
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("inspect 出力の JSON パースに失敗: {e}"))?;
    let root = json.value();

    let mut video_sample_count = 0;
    let mut decoded_320x320_count = 0;
    for sample in root.to_member("video_samples")?.required()?.to_array()? {
        video_sample_count += 1;
        assert!(
            sample.to_member("decoded_data_size")?.optional().is_some(),
            "全サンプルに decoded_data_size が付くこと (sample #{video_sample_count})"
        );
        let width = required_u64_member(sample, "width")?;
        let height = required_u64_member(sample, "height")?;
        if width == 320 && height == 320 {
            decoded_320x320_count += 1;
        }
    }

    assert_eq!(
        video_sample_count, 50,
        "映像サンプル数 50 (回帰検出アンカー)"
    );
    assert_eq!(
        decoded_320x320_count, 25,
        "後半 25 サンプルが 320x320 でデコードされること (回帰検出アンカー)"
    );
    Ok(())
}

fn required_u64_member(value: nojson::RawJsonValue<'_, '_>, key: &str) -> noargs::Result<u64> {
    value
        .to_member(key)?
        .required()?
        .try_into()
        .map_err(|e| format!("member {key} must be integer: {e}").into())
}

fn required_bool_member(value: nojson::RawJsonValue<'_, '_>, key: &str) -> noargs::Result<bool> {
    value
        .to_member(key)?
        .required()?
        .try_into()
        .map_err(|e| format!("member {key} must be boolean: {e}").into())
}

fn inspect_stdout(path: &str) -> noargs::Result<String> {
    let output = run_hisui_command(&["inspect", path])?;
    String::from_utf8(output.stdout)
        .map_err(|e| format!("inspect stdout is not valid UTF-8: {e}").into())
}

/// inspect 出力の `format` と `audio_codec` / `video_codec` が期待どおりか確認する。
/// 各 codec は `None` を渡せば「キー自体が出力されないこと」を assert する。これにより
/// 「映像のみ」「音声のみ」テストで想定外のトラックが混入した場合も検出できる。
fn assert_inspect_format_and_codec(
    stdout: &str,
    expected_format: &str,
    expected_audio_codec: Option<&str>,
    expected_video_codec: Option<&str>,
) -> noargs::Result<()> {
    let json = nojson::RawJson::parse(stdout)
        .map_err(|e| format!("inspect 出力の JSON パースに失敗: {e}"))?;
    let root = json.value();
    assert_eq!(
        root.to_member("format")?
            .required()?
            .to_unquoted_string_str()?,
        expected_format,
        "format が期待値と一致すること"
    );
    match expected_audio_codec {
        Some(expected) => assert_eq!(
            root.to_member("audio_codec")?
                .required()?
                .to_unquoted_string_str()?,
            expected,
            "audio_codec が期待値と一致すること"
        ),
        None => assert!(
            root.to_member("audio_codec")?.optional().is_none(),
            "音声トラックが無いはずなのに audio_codec キーが含まれている"
        ),
    }
    match expected_video_codec {
        Some(expected) => assert_eq!(
            root.to_member("video_codec")?
                .required()?
                .to_unquoted_string_str()?,
            expected,
            "video_codec が期待値と一致すること"
        ),
        None => assert!(
            root.to_member("video_codec")?.optional().is_none(),
            "映像トラックが無いはずなのに video_codec キーが含まれている"
        ),
    }
    Ok(())
}

/// 通常 MP4 と fMP4 で実値が一致するはずの inspect 出力フィールドだけを抽出した
/// 比較用構造体。`audio` / `video` の `None` は対応するトラックが inspect 出力に
/// 存在しないことを表す（`Some(Vec::new())` は「キーは存在するが要素 0」）。
///
/// 比較から除外する項目:
/// - `timestamp_us` / `duration_us`: 「音声+映像」ペアの映像トラックで testdata 生成差によりずれるため
/// - 集計値の `video_duration_us` / `audio_duration_us`: 同上
///
/// 映像サンプルの `nalus` は H.264 出力時のみ inspect が出力する。よって本構造体は
/// H.264 testdata 専用であり、H.265 / VP9 / AV1 の testdata では
/// `extract_inspect_comparable_samples` がパースエラーになる。
#[derive(Debug, PartialEq, Eq)]
struct InspectComparableSamples {
    audio: Option<Vec<InspectComparableAudioSample>>,
    video: Option<Vec<InspectComparableVideoSample>>,
}

#[derive(Debug, PartialEq, Eq)]
struct InspectComparableAudioSample {
    data_size: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct InspectComparableVideoSample {
    data_size: u64,
    keyframe: bool,
    nalus: Vec<InspectComparableNalu>,
}

/// `nalu_type` は JSON 出力上のキー名 `type`（Rust の予約語回避でリネーム）。
/// `nri` は H.264 の `nal_ref_idc`。
#[derive(Debug, PartialEq, Eq)]
struct InspectComparableNalu {
    nalu_type: u64,
    nri: u64,
}

/// inspect の出力 JSON 文字列から `InspectComparableSamples` を抽出する。
/// 詳細は `InspectComparableSamples` の doc を参照。
fn extract_inspect_comparable_samples(stdout: &str) -> noargs::Result<InspectComparableSamples> {
    let json = nojson::RawJson::parse(stdout)
        .map_err(|e| format!("inspect 出力の JSON パースに失敗: {e}"))?;
    let root = json.value();

    let audio = if let Some(audio_samples) = root.to_member("audio_samples")?.optional() {
        let mut out = Vec::new();
        for sample in audio_samples.to_array()? {
            out.push(InspectComparableAudioSample {
                data_size: required_u64_member(sample, "data_size")?,
            });
        }
        Some(out)
    } else {
        None
    };

    let video = if let Some(video_samples) = root.to_member("video_samples")?.optional() {
        let mut out = Vec::new();
        for sample in video_samples.to_array()? {
            let mut nalus = Vec::new();
            for nalu in sample.to_member("nalus")?.required()?.to_array()? {
                nalus.push(InspectComparableNalu {
                    nalu_type: required_u64_member(nalu, "type")?,
                    nri: required_u64_member(nalu, "nri")?,
                });
            }
            out.push(InspectComparableVideoSample {
                data_size: required_u64_member(sample, "data_size")?,
                keyframe: required_bool_member(sample, "keyframe")?,
                nalus,
            });
        }
        Some(out)
    } else {
        None
    };

    Ok(InspectComparableSamples { audio, video })
}
