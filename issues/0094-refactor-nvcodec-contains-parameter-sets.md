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

なお、length prefix ベースの全 NALU 走査による SPS / PPS / VPS の抽出は、既に共通ロジックとして `src/video/h264.rs` の `extract_h264_sps_pps_from_avcc` と `src/video/h265.rs` の `extract_h265_vps_sps_pps_from_avcc` に実装済みである。本 issue の機能をゼロから実装するのではなく、この共通関数を再利用する (§設計方針 参照)。

## 設計方針

`contains_parameter_sets` の「length prefix ベースで全 NALU を走査し、SPS (H.264) / SPS / PPS / VPS (H.265) の存在を判定する」機能は、既に共通の NAL 走査ロジックとして `src/video/h264.rs` の `extract_h264_sps_pps_from_avcc` と `src/video/h265.rs` の `extract_h265_vps_sps_pps_from_avcc` に実装済みである。

`contains_parameter_sets` はこの共通ロジックを再発明しているだけでなく、先頭 1 NALU しか見ない不完全な実装になっている。そこで nvcodec.rs 内で新たに自前の全 NALU 走査を書くのではなく、**共通関数を再利用して重複を排除する**。

## 完了条件

- SEI 先行 / AUD 先行の keyframe でも SPS / PPS の存在を正しく検出できる
- 単体テスト (`#[cfg(test)] mod tests`) で以下 4 パターンを最低限カバーする
  - 先頭が SPS
  - 先頭が SEI、後続に SPS
  - 先頭が AUD、後続に SPS
  - SPS がまったく含まれない (`false` を返す)

## 解決方法

- `contains_parameter_sets` を `bool` ではなく `crate::Result<bool>` を返す形に変更し、内部で共通関数を呼ぶ
  - `VideoFormat::H264`: `extract_h264_sps_pps_from_avcc` の `sps` / `pps` がどちらかでも `Some` なら `true`
  - `VideoFormat::H265`: `extract_h265_vps_sps_pps_from_avcc` の `vps` / `sps` / `pps` がどれかでも `Some` なら `true`
  - それ以外の形式 (`Av1` / `Vp8` / `Vp9` 等) は `false`
- 共通関数は壊れたフレーム (長さプレフィックスがデータ末尾を超える) を `Err` で返す。呼び出し側 (`src/decoder/nvcodec.rs` の Annex B 変換ループ前のパラメータセット prepend 分岐) で `?` により `Err` を巻き上げる。後段の Annex B 変換ループも同様に壊れたデータを `Err` にするため、`Err` を巻き上げるのが整合的
- nvcodec.rs で `contains_parameter_sets` 専用に使われていた `H264_NALU_TYPE_*` / `H265_NALU_TYPE_*` のインポートは不要になるため削除する (`NALU_HEADER_LENGTH` は Annex B 変換ループで引き続き使用するため残す)
- 既存の `contains_parameter_sets_*` テストを `Result<bool>` 対応に修正し、SEI 先行 / AUD 先行 / SPS なしのパターンを追加する

## 実装時の決定事項 (2026-08-17)

- **テストデータの AVCC 形式修正**: 旧テストは「長さ 1 の NALU の後ろに余分なバイト」を持つ (`[0,0,0,1, 0x67, 0x42]` 等) 壊れたデータを使っていた。旧実装は先頭 1 バイトだけ見るため気付かなかったが、共通関数は AVCC 構造を厳密に走査するため末尾の余分バイトを `Err` (truncated) として検出する。テストデータを正しい AVCC 形式 (各 NALU = 長さプレフィックス + データのみ) に修正した
- **`short_buffer` テストの意図変更**: `&[0,0,0,1]` (長さプレフィックスのみで NALU データ不足) は「SPS なし」ではなく「壊れたデータ」であり、共通関数は `Err` を返す。従来の `contains_parameter_sets_short_buffer_returns_false` は空バッファ (`&[]`) のみの false 確認に絞り、壊れたデータの `Err` 検証は新規テスト `contains_parameter_sets_truncated_returns_err` として分離した
