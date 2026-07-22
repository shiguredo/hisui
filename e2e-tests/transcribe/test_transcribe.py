"""`hisui -x transcribe` の e2e テスト。

Whisper と Silero VAD の実モデルを使い、Common Voice の CC0 短発話 (Opus in MP4) を
文字起こしして、標準出力の JSON LINE のスキーマと語彙を検証する。

環境変数 `HISUI_ML_MODELS_DIR` が model 配置ディレクトリを指す必要がある
(CI では `${{ github.workspace }}/ml-models` を設定する)。 未設定なら pytest.skip する
(ローカル環境で ML モデル未配置なら実行しない)。
"""

import json
import os
import subprocess
from pathlib import Path

import pytest

from hisui_server import REPO_ROOT, build_hisui_command


def _ml_models_dir() -> Path | None:
    """`HISUI_ML_MODELS_DIR` から model ディレクトリを解決する。 未設定なら None。"""
    value = os.environ.get("HISUI_ML_MODELS_DIR")
    if not value:
        return None
    return Path(value)


def _whisper_model_dir() -> Path | None:
    root = _ml_models_dir()
    if root is None:
        return None
    return root / "whisper-tiny"


def _silero_vad_model() -> Path | None:
    root = _ml_models_dir()
    if root is None:
        return None
    return root / "silero-vad" / "onnx" / "model.onnx"


def _fixture_path(name: str) -> Path:
    return REPO_ROOT / "testdata" / "e2e" / "transcribe" / name


def _skip_if_models_missing() -> tuple[Path, Path]:
    """モデルが揃っていなければ pytest.skip する。 揃っていれば (whisper_dir, silero_path) を返す。"""
    whisper_dir = _whisper_model_dir()
    silero_path = _silero_vad_model()
    if whisper_dir is None or silero_path is None:
        pytest.skip("HISUI_ML_MODELS_DIR が未設定のため skip する")
    if not whisper_dir.is_dir():
        pytest.skip(f"Whisper モデルディレクトリが見つからないため skip する: {whisper_dir}")
    if not silero_path.is_file():
        pytest.skip(f"Silero VAD モデルファイルが見つからないため skip する: {silero_path}")
    return whisper_dir, silero_path


def _run_transcribe(binary_path: Path, language: str, fixture: str) -> list[dict]:
    """`hisui -x transcribe` を起動して stdout の JSON LINE を list に返す。"""
    whisper_dir, silero_path = _skip_if_models_missing()
    fixture_path = _fixture_path(fixture)
    assert fixture_path.is_file(), f"fixture が見つからない: {fixture_path}"

    command, cwd = build_hisui_command(
        binary_path,
        "-x",
        "transcribe",
        "--model-dir",
        str(whisper_dir),
        "--silero-vad-model",
        str(silero_path),
        "--language",
        language,
        str(fixture_path),
    )
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=False)
    assert result.returncode == 0, (
        f"transcribe が非ゼロ exit code で終了した: rc={result.returncode}\n"
        f"stderr:\n{result.stderr}"
    )

    lines: list[dict] = []
    for raw in result.stdout.splitlines():
        raw = raw.strip()
        if not raw:
            continue
        lines.append(json.loads(raw))
    assert lines, f"少なくとも 1 行の JSON LINE が出力されるはず: stdout=\n{result.stdout}"
    return lines


def _is_japanese_char(c: str) -> bool:
    """ひらがな / カタカナ / CJK 統合漢字のいずれかなら True。"""
    code = ord(c)
    return (
        0x3040 <= code <= 0x309F  # ひらがな
        or 0x30A0 <= code <= 0x30FF  # カタカナ
        or 0x4E00 <= code <= 0x9FFF  # CJK 統合漢字
    )


def _assert_common_schema(lines: list[dict], expected_language: str) -> None:
    """全 JSON LINE に共通する制約を検証する。"""
    prev_end = -1.0
    for i, line in enumerate(lines):
        for key in ("start", "end", "text"):
            assert key in line, f"line {i}: 必須キー {key} が無い: {line}"
        assert isinstance(line["start"], (int, float)), f"line {i}: start が数値でない"
        assert isinstance(line["end"], (int, float)), f"line {i}: end が数値でない"
        assert line["start"] <= line["end"], (
            f"line {i}: start <= end であるべき: start={line['start']}, end={line['end']}"
        )
        assert line["start"] >= prev_end, (
            f"line {i}: start は非減少 (単調増加相当) であるべき: "
            f"start={line['start']}, prev_end={prev_end}"
        )
        prev_end = line["end"]
        assert line.get("language") == expected_language, (
            f"line {i}: language は {expected_language} であるべき: {line.get('language')}"
        )
        for key in ("no_speech_prob", "avg_logprob"):
            if key in line:
                assert isinstance(line[key], (int, float)), f"line {i}: {key} が数値でない"


@pytest.mark.timeout(120)
def test_transcribe_english_fixture(binary_path: Path) -> None:
    """英語 fixture (`speech-en.mp4`) を transcribe すると英字を含む JSON LINE が返る。"""
    lines = _run_transcribe(binary_path, "en", "speech-en.mp4")
    _assert_common_schema(lines, "en")

    # 少なくとも 1 行に非空 text + 英字が含まれること
    ascii_letters_total = 0
    for line in lines:
        text = line["text"]
        assert isinstance(text, str), f"text が str でない: {line}"
        ascii_letters_total += sum(1 for c in text if c.isascii() and c.isalpha())
    assert ascii_letters_total >= 3, (
        f"英語 fixture の文字起こしは英字を十分含むこと: {[line['text'] for line in lines]}"
    )

    # 品質指標の緩い閾値 (Whisper integration test と同じ値)
    for line in lines:
        if "no_speech_prob" in line:
            assert line["no_speech_prob"] < 0.5, (
                f"no_speech_prob は 0.5 未満のはず: {line['no_speech_prob']}"
            )
        if "avg_logprob" in line:
            assert line["avg_logprob"] > -1.5, (
                f"avg_logprob は -1.5 より大きいはず: {line['avg_logprob']}"
            )


@pytest.mark.timeout(120)
def test_transcribe_japanese_fixture(binary_path: Path) -> None:
    """日本語 fixture (`speech-ja.mp4`) を transcribe すると日本語文字を含む JSON LINE が返る。"""
    lines = _run_transcribe(binary_path, "ja", "speech-ja.mp4")
    _assert_common_schema(lines, "ja")

    # 少なくとも 1 行に非空 text + 日本語文字が含まれること
    japanese_chars_total = 0
    for line in lines:
        text = line["text"]
        japanese_chars_total += sum(1 for c in text if _is_japanese_char(c))
    assert japanese_chars_total >= 3, (
        f"日本語 fixture の文字起こしは日本語文字を十分含むこと: {[line['text'] for line in lines]}"
    )

    for line in lines:
        if "no_speech_prob" in line:
            assert line["no_speech_prob"] < 0.5
        if "avg_logprob" in line:
            assert line["avg_logprob"] > -1.5


@pytest.mark.timeout(60)
def test_transcribe_without_experimental_flag_fails(binary_path: Path) -> None:
    """`--experimental` (`-x`) 無しで `transcribe` を呼ぶと非ゼロ exit で終了する。"""
    _skip_if_models_missing()
    fixture_path = _fixture_path("speech-en.mp4")
    whisper_dir = _whisper_model_dir()
    silero_path = _silero_vad_model()

    command, cwd = build_hisui_command(
        binary_path,
        "transcribe",
        "--model-dir",
        str(whisper_dir),
        "--silero-vad-model",
        str(silero_path),
        "--language",
        "en",
        str(fixture_path),
    )
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=False)
    assert result.returncode != 0, "--experimental 無しは非ゼロ exit code で終了するはず"
    # 標準エラーに日本語メッセージが含まれる
    assert "実験的機能" in result.stderr, f"stderr に「実験的機能」を含むこと: {result.stderr}"
