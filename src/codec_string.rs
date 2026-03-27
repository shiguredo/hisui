/// HLS / DASH マニフェストに記載するコーデック文字列。
///
/// HLS のマスタープレイリスト（`CODECS` 属性）や DASH の MPD（`codecs` 属性）で使われる。
/// エンコーダーの設定に合わせて coordinator が構築し、各ライターに渡す。
#[derive(Debug, Clone)]
pub struct CodecString {
    /// ビデオコーデック文字列（例: "avc1.42e01f"）
    pub video: String,
    /// オーディオコーデック文字列（例: "mp4a.40.2"）
    pub audio: String,
}

impl CodecString {
    /// H.264 Baseline Profile Level 3.1 + AAC-LC のデフォルト値。
    ///
    /// 現在 hisui のライブ出力は H.264 + AAC 固定のため、これを使用する。
    /// 将来エンコーダーの SPS/AudioSpecificConfig から正確な値を取得する場合は、
    /// 別のコンストラクタを追加する。
    pub fn h264_aac_default() -> Self {
        Self {
            video: "avc1.42e01f".to_owned(),
            audio: "mp4a.40.2".to_owned(),
        }
    }

    /// "video_codec,audio_codec" 形式の結合文字列を返す。
    /// HLS の CODECS 属性や DASH の codecs 属性にそのまま使える。
    pub fn as_combined(&self) -> String {
        format!("{},{}", self.video, self.audio)
    }
}
