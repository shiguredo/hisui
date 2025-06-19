use orfail::OrFail;

pub fn run(args: noargs::RawArgs) -> noargs::Result<()> {
    // 引数の解析を完了する（このサブコマンドは追加の引数を取らない）
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    // 利用可能なコーデック情報を収集
    let mut codecs = Vec::new();

    // 音声コーデック
    codecs.push(CodecInfo {
        name: "Opus".to_string(),
        codec_type: "audio".to_string(),
        available: true,
        description: "Opus audio codec".to_string(),
    });

    #[cfg(any(target_os = "macos", feature = "fdk-aac"))]
    codecs.push(CodecInfo {
        name: "AAC".to_string(),
        codec_type: "audio".to_string(),
        available: true,
        description: "AAC audio codec".to_string(),
    });

    #[cfg(not(any(target_os = "macos", feature = "fdk-aac")))]
    codecs.push(CodecInfo {
        name: "AAC".to_string(),
        codec_type: "audio".to_string(),
        available: false,
        description: "AAC audio codec (requires macOS or FDK-AAC feature)".to_string(),
    });

    // 映像コーデック
    codecs.push(CodecInfo {
        name: "VP8".to_string(),
        codec_type: "video".to_string(),
        available: true,
        description: "VP8 video codec".to_string(),
    });

    codecs.push(CodecInfo {
        name: "VP9".to_string(),
        codec_type: "video".to_string(),
        available: true,
        description: "VP9 video codec".to_string(),
    });

    codecs.push(CodecInfo {
        name: "H264".to_string(),
        codec_type: "video".to_string(),
        available: true,
        description: "H.264 video codec".to_string(),
    });

    #[cfg(target_os = "macos")]
    codecs.push(CodecInfo {
        name: "H265".to_string(),
        codec_type: "video".to_string(),
        available: true,
        description: "H.265 video codec (macOS only)".to_string(),
    });

    #[cfg(not(target_os = "macos"))]
    codecs.push(CodecInfo {
        name: "H265".to_string(),
        codec_type: "video".to_string(),
        available: false,
        description: "H.265 video codec (macOS only)".to_string(),
    });

    codecs.push(CodecInfo {
        name: "AV1".to_string(),
        codec_type: "video".to_string(),
        available: true,
        description: "AV1 video codec".to_string(),
    });

    // JSON形式で出力
    println!(
        "{}",
        nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.object(|f| f.member("codecs", &codecs))
        })
    );

    Ok(())
}

#[derive(Debug)]
struct CodecInfo {
    name: String,
    codec_type: String,
    available: bool,
    description: String,
}

impl nojson::DisplayJson for CodecInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("name", &self.name)?;
            f.member("type", &self.codec_type)?;
            f.member("available", self.available)?;
            f.member("description", &self.description)
        })
    }
}
