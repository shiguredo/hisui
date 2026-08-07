# nvcodec デコーダーの contains_parameter_sets が先頭 1 NALU しか判定しない

- Created: 2026-08-07
- Branch: feature/refactor-nvcodec-contains-parameter-sets

## 目的

`src/decoder/nvcodec.rs` の `contains_parameter_sets` を length prefix ベースの全 NALU 走査に変えて、SEI / AUD 先行時にも SPS / PPS の存在を正しく検出できるようにする。他ソース由来の H.264 / H.265 入力への堅牢性を上げる目的。

## 現状

`contains_parameter_sets` は length prefix (4 バイト) をスキップした先頭 1 NALU の 1 バイト目だけを見て SPS / PPS / VPS の存在を判定する:

```rust
fn contains_parameter_sets(data: &[u8], format: VideoFormat) -> bool {
    if data.len() < NALU_HEADER_LENGTH + 1 {
        return false;
    }
    match format {
        VideoFormat::H265 => {
            let nal_unit_type = (data[NALU_HEADER_LENGTH] >> 1) & 0x3F;
            matches!(nal_unit_type, H265_NALU_TYPE_PPS | H265_NALU_TYPE_SPS | H265_NALU_TYPE_VPS)
        }
        VideoFormat::H264 => {
            let nal_unit_type = data[NALU_HEADER_LENGTH] & 0x1F;
            matches!(nal_unit_type, H264_NALU_TYPE_SPS | H264_NALU_TYPE_PPS)
        }
        VideoFormat::Av1 => false,
        _ => false,
    }
}
```

フレームデータが `[AUD][SPS][PPS][IDR]` や `[SEI][SPS][PPS][IDR]` の順で来た場合、先頭 NALU が SPS / PPS / VPS のいずれでもないため `false` を返し、キャッシュされた parameter_sets が二重に prepend される。NVDEC は冗長な SPS / PPS を許容するため decode 自体は壊れないが、H.265 では VPS の重複が parser の余計な sequence change 検出を招くリスクがある。

Sora の H.264 / H.265 録画では通常 SEI / AUD が先頭に来ないため実データでは発生していないが、Sora 以外のソース由来の入力を扱う場合に堅牢性が不足する。

## 設計方針

length prefix ベースで全 NALU をループ走査し、少なくとも 1 個の SPS (H.264 / H.265 では PPS / VPS も含めた判定) が存在するかを判定する。

## 完了条件

- SEI 先行 / AUD 先行の keyframe でも SPS / PPS の存在を正しく検出できる
- 単体テスト (`#[cfg(test)] mod tests`) で以下 4 パターンを最低限カバーする
  - 先頭が SPS
  - 先頭が SEI、後続に SPS
  - 先頭が AUD、後続に SPS
  - SPS がまったく含まれない (`false` を返す)

## 解決方法

- `contains_parameter_sets` を length prefix ベースの NALU 単位走査に書き換える
- 走査中に SPS (H.264) / SPS / PPS / VPS (H.265) を検出したら `true` を返す
- 単体テストを追加してカバレッジを確保する
