# デコード設定の指定方法

[レイアウト機能](./layout.md) で指定する JSON ファイルでは、
映像ソースをデコードする一部のデコーダーのパラメーターを指定することができます。

デコードパラメーターによって、デコード性能やメモリ使用量が変わる可能性があるため、
環境に適した設定を行うことで、より効率的な合成処理が可能になります。

なお、デフォルトで使用されるデコードパラメーターは [layout-examples/compose-default.jsonc](../layout-examples/compose-default.jsonc) にも記載されています。

## NVIDIA Video Codec SDK デコーダーパラメーター

NVIDIA Video Codec SDK（nvcodec）は、CUDA 対応 GPU を利用したハードウェアデコーダーです。
Hisui では H.264、H.265、AV1 のデコードに利用できます。

### 利用条件

nvcodec デコーダーを利用するには、以下の条件を満たしている必要があります：

- CUDA 対応の NVIDIA GPU
- 適切な NVIDIA GPU ドライバー
- nvcodec に対応してビルドされた Hisui（[ビルド方法](./build.md)を参照）

### パラメーター

nvcodec デコーダーでは、H.264、H.265、AV1 の全てのコーデックで共通のパラメーターを利用できます。

これらのパラメーターは、それぞれ以下のキーで指定します：

- `nvcodec_h264_decode_params`: H.264 デコード時のパラメーター
- `nvcodec_h265_decode_params`: H.265 デコード時のパラメーター
- `nvcodec_av1_decode_params`: AV1 デコード時のパラメーター

なお、本ドキュメントでの各パラメーターについての説明などは参考程度のものとなっております。
正確な情報については、公式ドキュメントを参照してください。

#### デバイス制御パラメーター

- `device_id` (整数値): 使用する CUDA デバイスの ID
  - デフォルト値: `0`
  - 指定可能な範囲: 0 以上の値（システムの利用可能 GPU 数による）

#### メモリ・バッファ制御パラメーター

- `max_num_decode_surfaces` (整数値): デコード用サーフェスの最大数
  - デフォルト値: `20`
  - 指定可能な範囲: 1 以上の値
  - 値を大きくするとメモリ使用量が増加しますが、デコード性能が向上する可能性があります

- `max_display_delay` (整数値): 表示遅延の最大フレーム数
  - デフォルト値: `0`
  - 指定可能な範囲: 0以上の値
  - 0に設定すると遅延を最小にし、値を大きくするとデコード効率が向上します

## 使用例

以下は、nvcodec デコーダーパラメーターを指定したレイアウトファイルの例です：

```json
{
  "video_codec": "H264",
  "video_layout": {
    "main": {
      "video_sources": ["archive-*.json"]
    }
  },
  "nvcodec_h264_decode_params": {
    "device_id": 0,
    "max_num_decode_surfaces": 16,
    "max_display_delay": 1
  }
}

