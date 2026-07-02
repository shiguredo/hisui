//! Silero VAD 実推論による integration テスト。
//!
//! - `HISUI_ML_MODELS_DIR` (未設定時のデフォルトは `ml-models`) の下から `silero-vad/onnx/model.onnx`
//!   を読む
//! - モデルファイルが存在しない場合は `println!` で理由を出して skip する
//! - ただし CI で silent skip されて仕様退行に気付けなくならないよう、環境変数 `HISUI_CI=1` が
//!   設定されている場合は skip せず panic する

#![cfg(feature = "candle")]

use std::path::{Path, PathBuf};

use candle_core::Device;
use hisui::ml::audio::{SileroVad, VadConfig, VadGate};

/// モデル配置ディレクトリを環境変数から解決する。未設定なら `ml-models` を返す。
fn ml_models_dir() -> PathBuf {
    std::env::var("HISUI_ML_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("ml-models"))
}

/// Silero VAD モデルのパスを返す。ファイル不在で skip すべき場合は None。
///
/// `HISUI_CI=1` が設定されているときは skip せず panic する (CI での silent skip 防止)。
fn resolve_model_path_or_skip(test_name: &str) -> Option<PathBuf> {
    let path = ml_models_dir().join("silero-vad/onnx/model.onnx");
    if path.is_file() {
        return Some(path);
    }
    if std::env::var("HISUI_CI").as_deref() == Ok("1") {
        panic!(
            "HISUI_CI=1 だが Silero VAD モデルが見つからない: {} (test={test_name})",
            path.display()
        );
    }
    println!(
        "skip {test_name}: Silero VAD モデルが見つからない (HISUI_ML_MODELS_DIR={:?}, 解決先={})",
        ml_models_dir(),
        path.display()
    );
    None
}

/// SileroVad::load が成功する (モデル配置済み環境限定)。
#[test]
fn silero_vad_load_succeeds() {
    let Some(model_path) = resolve_model_path_or_skip("silero_vad_load_succeeds") else {
        return;
    };
    SileroVad::load(&model_path, Device::Cpu).expect("Silero VAD モデルのロードは成功する想定");
}

/// 3 秒の zero-fill を VadGate に流すと SpeechSegment が空 Vec で返る。
#[test]
fn vad_gate_returns_no_segment_for_zero_fill() {
    let Some(model_path) = resolve_model_path_or_skip("vad_gate_returns_no_segment_for_zero_fill")
    else {
        return;
    };
    let silero = SileroVad::load(&model_path, Device::Cpu).expect("Silero VAD ロード");
    let mut gate = VadGate::new(silero, VadConfig::default());

    // 3 秒 = 48000 サンプル @ 16 kHz の zero-fill。
    let pcm = vec![0.0f32; 48000];
    let segments = gate.feed(&pcm).expect("zero-fill は Err を返さないはず");
    assert!(
        segments.is_empty(),
        "無音では SpeechSegment が返らないはず: {segments:?}"
    );

    let flushed = gate.flush().expect("flush は Err を返さないはず");
    assert!(
        flushed.is_empty(),
        "無音のみなら flush でも SpeechSegment は返らないはず: {flushed:?}"
    );
}

/// 512 サンプル zero-fill を 3 回 SileroVad::chunk_probability に流すと、返る確率が全て閾値 (0.5)
/// 未満かつ 3 回とも等しい (state / context が初期状態でゼロ入力なので結果は決定的)。
#[test]
fn silero_vad_zero_input_is_deterministic_and_below_threshold() {
    let Some(model_path) =
        resolve_model_path_or_skip("silero_vad_zero_input_is_deterministic_and_below_threshold")
    else {
        return;
    };
    let mut silero = SileroVad::load(&model_path, Device::Cpu).expect("Silero VAD ロード");
    let chunk = vec![0.0f32; 512];
    let p1 = silero
        .chunk_probability(&chunk)
        .expect("chunk_probability は Ok");
    let p2 = silero
        .chunk_probability(&chunk)
        .expect("chunk_probability は Ok");
    let p3 = silero
        .chunk_probability(&chunk)
        .expect("chunk_probability は Ok");
    assert!(p1 < 0.5, "無音チャンクの確率は閾値 0.5 未満のはず: {p1}");
    assert!(p2 < 0.5, "無音チャンクの確率は閾値 0.5 未満のはず: {p2}");
    assert!(p3 < 0.5, "無音チャンクの確率は閾値 0.5 未満のはず: {p3}");
    // reset していないので state / context は更新されているが、入力がゼロならモデル出力も 3 回目までは
    // 決定的に同一値のはず (数値的な同一性を確認する)。
    assert_eq!(p1, p2, "同一 zero 入力で 2 回目の確率は同じ想定");
    assert_eq!(p2, p3, "同一 zero 入力で 3 回目の確率は同じ想定");
}

/// パス不在で SileroVad::load が Err を返す。
///
/// このケースはモデル配置に依存しないため、常時実行する。
#[test]
fn silero_vad_load_returns_err_for_missing_path() {
    let missing = Path::new("/nonexistent/silero-vad/model.onnx");
    let err = SileroVad::load(missing, Device::Cpu).expect_err("存在しないパスは Err を返す想定");
    let msg = err.display().to_string();
    assert!(
        msg.contains("not found") || msg.contains("silero VAD"),
        "エラーメッセージに 'not found' または 'silero VAD' を含むこと: {msg}"
    );
}

/// 非 ONNX バイト列で SileroVad::load が Err を返す。
///
/// magic bytes を持たない 32 byte を tempfile に書いて load すると、candle-onnx が ONNX として
/// パースできず Err になる。
#[test]
fn silero_vad_load_returns_err_for_non_onnx_bytes() {
    let dir = std::env::temp_dir().join(format!(
        "hisui_test_silero_vad_bad_bytes_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("tempdir 作成");
    let path = dir.join("bad.onnx");
    let bad_bytes = b"NOT_ONNX_FILE_HEADER_DATA_XXXXXX"; // 32 byte、非 protobuf
    std::fs::write(&path, bad_bytes).expect("bad bytes 書き込み");

    let err = SileroVad::load(&path, Device::Cpu).expect_err("非 ONNX バイト列は Err を返す想定");

    // 後片付け。
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);

    let msg = err.display().to_string();
    assert!(
        msg.contains("parse") || msg.contains("ONNX") || msg.contains("graph"),
        "エラーメッセージにパース系の語を含むこと: {msg}"
    );
}
