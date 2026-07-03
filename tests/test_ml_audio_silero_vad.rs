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
use hisui::ml::audio::config::VadConfig;
use hisui::ml::audio::silero_vad::SileroVadModel;
use hisui::ml::audio::vad::VadGate;

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

/// SileroVadModel::load が成功する (モデル配置済み環境限定)。
#[test]
fn silero_vad_model_load_succeeds() {
    let Some(model_path) = resolve_model_path_or_skip("silero_vad_model_load_succeeds") else {
        return;
    };
    SileroVadModel::load(&model_path, Device::Cpu)
        .expect("Silero VAD モデルのロードは成功する想定");
}

/// 3 秒の zero-fill を VadGate に流すと SpeechSegment が空 Vec で返る。
#[test]
fn vad_gate_returns_no_segment_for_zero_fill() {
    let Some(model_path) = resolve_model_path_or_skip("vad_gate_returns_no_segment_for_zero_fill")
    else {
        return;
    };
    let model = SileroVadModel::load(&model_path, Device::Cpu).expect("Silero VAD ロード");
    let mut gate = VadGate::new(model.new_instance(), VadConfig::default());

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

/// 512 サンプル zero-fill を 3 回 SileroVad::chunk_probability に流すと、返る確率がいずれも
/// 閾値 (0.5) 未満に収まる。
///
/// Silero VAD v5 は LSTM state を推論間で持ち回すため、同じ入力でも 2 回目以降の確率は
/// 1 回目と一致しない (state が更新されるため)。「決定的に同一値」を assert してはいけない。
#[test]
fn silero_vad_zero_input_stays_below_threshold() {
    let Some(model_path) =
        resolve_model_path_or_skip("silero_vad_zero_input_stays_below_threshold")
    else {
        return;
    };
    let model = SileroVadModel::load(&model_path, Device::Cpu).expect("Silero VAD ロード");
    let mut silero = model.new_instance();
    let chunk = vec![0.0f32; 512];
    for i in 0..3 {
        let probability = silero
            .chunk_probability(&chunk)
            .expect("chunk_probability は Ok");
        assert!(
            probability < 0.5,
            "{i} 回目の無音チャンクの確率は閾値 0.5 未満のはず: {probability}"
        );
    }
}

/// 同じモデルから `new_instance` で作った独立したインスタンスは、同一入力に対して 1 回目の推論結果が
/// 一致する (両者とも初期 state から始まるため決定的に同値)。
#[test]
fn independent_instances_produce_identical_first_probability() {
    let Some(model_path) =
        resolve_model_path_or_skip("independent_instances_produce_identical_first_probability")
    else {
        return;
    };
    let model = SileroVadModel::load(&model_path, Device::Cpu).expect("Silero VAD ロード");
    let mut vad_a = model.new_instance();
    let mut vad_b = model.new_instance();

    let chunk = vec![0.0f32; 512];
    let prob_a = vad_a
        .chunk_probability(&chunk)
        .expect("A: chunk_probability は Ok");
    let prob_b = vad_b
        .chunk_probability(&chunk)
        .expect("B: chunk_probability は Ok");
    assert_eq!(
        prob_a, prob_b,
        "独立インスタンスは初期 state から始まるので 1 回目の確率は一致するはず"
    );
}

/// A の state を進めた後でも、独立している B は影響を受けない (state 分離の検証)。
#[test]
fn instances_do_not_share_state() {
    let Some(model_path) = resolve_model_path_or_skip("instances_do_not_share_state") else {
        return;
    };
    let model = SileroVadModel::load(&model_path, Device::Cpu).expect("Silero VAD ロード");
    let mut vad_a = model.new_instance();
    let mut vad_b = model.new_instance();

    let chunk = vec![0.0f32; 512];

    // A を 5 チャンクぶん進めて state を変える。
    for _ in 0..5 {
        vad_a
            .chunk_probability(&chunk)
            .expect("A: chunk_probability は Ok");
    }

    // B は初期 state のままなので、1 回目 (=独立に作ったばかりの結果) は
    // 前テストで確認した「初期 state での確率」と一致する。改めて別の fresh インスタンスと比較する。
    let prob_b_first = vad_b
        .chunk_probability(&chunk)
        .expect("B: chunk_probability は Ok");
    let mut vad_c = model.new_instance();
    let prob_c_first = vad_c
        .chunk_probability(&chunk)
        .expect("C: chunk_probability は Ok");
    assert_eq!(
        prob_b_first, prob_c_first,
        "A の state 変化は B に伝わっていないはず"
    );
}

/// パス不在で SileroVadModel::load が Err を返す。
///
/// このケースはモデル配置に依存しないため、常時実行する。
#[test]
fn silero_vad_model_load_returns_err_for_missing_path() {
    let missing = Path::new("/nonexistent/silero-vad/model.onnx");
    let err =
        SileroVadModel::load(missing, Device::Cpu).expect_err("存在しないパスは Err を返す想定");
    let msg = err.display().to_string();
    assert!(
        msg.contains("not found") || msg.contains("silero VAD"),
        "エラーメッセージに 'not found' または 'silero VAD' を含むこと: {msg}"
    );
}

/// 非 ONNX バイト列で SileroVadModel::load が Err を返す。
///
/// magic bytes を持たない 32 byte を tempfile に書いて load すると、candle-onnx が ONNX として
/// パースできず Err になる。
#[test]
fn silero_vad_model_load_returns_err_for_non_onnx_bytes() {
    let dir = std::env::temp_dir().join(format!(
        "hisui_test_silero_vad_bad_bytes_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("tempdir 作成");
    let path = dir.join("bad.onnx");
    let bad_bytes = b"NOT_ONNX_FILE_HEADER_DATA_XXXXXX"; // 32 byte、非 protobuf
    std::fs::write(&path, bad_bytes).expect("bad bytes 書き込み");

    let err =
        SileroVadModel::load(&path, Device::Cpu).expect_err("非 ONNX バイト列は Err を返す想定");

    // 後片付け。
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);

    let msg = err.display().to_string();
    assert!(
        msg.contains("parse") || msg.contains("ONNX") || msg.contains("graph"),
        "エラーメッセージにパース系の語を含むこと: {msg}"
    );
}
