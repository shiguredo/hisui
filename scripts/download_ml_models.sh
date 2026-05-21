#!/usr/bin/env bash
# ML デモ用モデル重みを Hugging Face から取得する。
#
# 使い方:
#   ./scripts/download_ml_models.sh [all|whisper|yolo|vad]
#
# 環境変数:
#   ML_MODELS_DIR  保存先（既定: リポジトリ直下の ml-models）

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${ML_MODELS_DIR:-$ROOT/ml-models}"

usage() {
    cat <<'EOF'
Usage: download_ml_models.sh [all|whisper|yolo|vad]

  all     whisper-tiny + silero-vad + YOLOv8s（既定）
  whisper openai/whisper-tiny（config.json, tokenizer.json, model.safetensors）
  vad     onnx-community/silero-vad（onnx/model.onnx）
  yolo    lmz/candle-yolo-v8 の yolov8s.safetensors と yolov8s-pose.safetensors

環境変数 ML_MODELS_DIR で保存先を変更できます（既定: ./ml-models）。
EOF
}

download_file() {
    local url="$1"
    local path="$2"
    if [[ -f "$path" ]]; then
        echo "skip (exists): $path"
        return
    fi
    mkdir -p "$(dirname "$path")"
    echo "download: $path"
    curl -fL --retry 3 --retry-delay 2 -o "$path" "$url"
}

download_whisper() {
    local dir="$DEST/whisper-tiny"
    local base="https://huggingface.co/openai/whisper-tiny/resolve/main"
    download_file "$base/config.json" "$dir/config.json"
    download_file "$base/tokenizer.json" "$dir/tokenizer.json"
    download_file "$base/model.safetensors" "$dir/model.safetensors"
    echo "whisper: $dir"
}

download_vad() {
    local dir="$DEST/silero-vad/onnx"
    local base="https://huggingface.co/onnx-community/silero-vad/resolve/main/onnx"
    download_file "$base/model.onnx" "$dir/model.onnx"
    echo "silero vad: $dir/model.onnx"
}

download_yolo() {
    local dir="$DEST/yolo"
    local base="https://huggingface.co/lmz/candle-yolo-v8/resolve/main"
    download_file "$base/yolov8s.safetensors" "$dir/yolov8s.safetensors"
    download_file "$base/yolov8s-pose.safetensors" "$dir/yolov8s-pose.safetensors"
    echo "yolo: $dir"
}

main() {
    local target="${1:-all}"
    case "$target" in
        all)
            download_whisper
            download_vad
            download_yolo
            ;;
        whisper)
            download_whisper
            ;;
        vad)
            download_vad
            ;;
        yolo)
            download_yolo
            ;;
        -h | --help | help)
            usage
            exit 0
            ;;
        *)
            echo "unknown target: $target" >&2
            usage >&2
            exit 1
            ;;
    esac

    cat <<EOF

Done. 例:

  hisui ml audio --model-dir $DEST/whisper-tiny --vad --vad-trim --verbose
  hisui ml --model-path $DEST/yolo/yolov8s.safetensors --list-devices
EOF
}

main "$@"
