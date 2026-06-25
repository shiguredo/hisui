# RTSP AU header パース用の独自 BitReader を共有 BitReader に統合する

- Priority: Low
- Created: 2026-06-24
- Completed: 2026-06-25
- Model: Opus 4.7
- Branch: feature/refactor-unify-rtsp-au-header-bit-reader
- Polished: 2026-06-25

## 目的

closed/0048 で `src/video/bit_reader.rs::BitReader` を codec 中立な汎用ビットリーダーとして新設し、H.264 / H.265 SPS パーサで共有するように整理した。一方で `src/rtsp/subscriber.rs` には RTP MPEG4-GENERIC AAC AU header をパースするための独自 `BitReader` 構造体が以前から存在しており、機能的にほぼ同等 (MSB ファースト / バッファ枯渇で Err) でありながら別実装になっている。`BitReader` という汎用名前空間を 2 箇所が確保している状態を解消する。

## 優先度根拠

Low。内部リファクタリングが主目的。RTSP 機能自体は `## develop` 内の未リリース機能のため、本 issue の副次的な挙動変化は外部リリース観点では「develop 内の中間状態の修正」に該当し CHANGES.md 記載は不要 (`shiguredo-changelog` 規約準拠)。

- subscriber 側の利用箇所は `AacRtpDepacketizer::depacketize` 1 箇所のみで影響範囲が狭い。
- closed/0048 の `### 残懸念` で本 issue を予告済み。片肺のまま放置するとレビュー時の認知負荷が継続する。

副次的な内部挙動変化 (develop 内に閉じる、CHANGES.md 不要):

- AU header パース失敗時のエラーメッセージが共有 BitReader 由来 (`"bit reader: exhausted before requested read"`) に変わる。`### エラーメッセージの粒度` 参照の上で `Error::with_context` 経由で context 付き文言 (`"invalid AAC AU header: bit reader: exhausted before requested read"` 等) に wrap し、AU header 経路と SPS パース経路の Err をログから切り分けられるようにする。
- fmtp の `sizelength` / `indexlength` / `indexdeltalength` のいずれかが 32 超のとき、現状は壊れた値を素通ししていたが、修正後は session 確立時点で Err になる (`### RFC 3640 §3.3.6 準拠の fmtp パラメータ検証` 参照)。実用上の RTSP 配信側で 32 超を送る実装は想定外。

## 現状

行番号は実装着手時に関連シンボルを grep で再特定する。本文では原則として関数名・型名で参照する。

### 2 つの BitReader の比較

| 項目 | `src/video/bit_reader.rs::BitReader` (pub) | `src/rtsp/subscriber.rs::BitReader` (非 pub) |
| --- | --- | --- |
| フィールド | `data: &[u8]` / `byte_pos: usize` / `bit_pos: u8` | `bytes: &[u8]` / `bit_offset: usize` |
| API | `read_u(n: usize) -> Result<u32>` / `read_ue` / `read_se` / `skip_u` / `skip_ue` / `skip_se` | `read_bits(bit_count: u8) -> Result<u32>` のみ |
| `n` 上限検査 | `n > 32` で Err | 無し (`bit_count` は u8 で 0..=255、`> 32` で `u32` の左シフトにより上位ビットが落ちた壊れた値を返す) |
| `bit_count == 0` の扱い | `for _ in 0..0` でループ 0 回 → `Ok(0)` | 早期 return で `Ok(0)` |
| 空バッファ + `bit_count == 0` | `Ok(0)` (read_bit を呼ばない) | `Ok(0)` (早期 return) |
| バッファ枯渇 | Err `"bit reader: exhausted before requested read"` | Err `"bitstream is truncated"` |
| MSB ファースト | はい | はい |

### 呼び出し箇所

`src/rtsp/subscriber.rs::AacRtpDepacketizer::depacketize` 内の AU header パースループで `BitReader::new(au_headers)` を 1 回構築し、`read_bits` を 2 種類のビット数で複数回呼んでいる (`size_length` と `index_length` または `index_delta_length`)。

### fmtp パラメータの取得経路

`size_length` / `index_length` / `index_delta_length` は SDP の fmtp から `parse::<u8>()` 経由で `AudioTrackConfig` に格納する (`select_audio_track` 内)。値域は 0..=255 で、現状の事前検査は以下のみ。

- `size_length == 0` → Err `"AAC fmtp sizeLength must be greater than 0"`

`size_length > 32` / `index_length > 32` / `index_delta_length > 32` の上限検査は無い。`index_length == 0` / `index_delta_length == 0` は RFC 3640 §3.3.6 で許容されているため、現状の暗黙的な合法値として残す。

### 確認済み事項 (grep / テスト)

- `grep -rn '"bitstream is truncated"' src/ tests/ pbt/` の結果、ヒットは `src/rtsp/subscriber.rs` の発生源 1 箇所のみ。assert 等で文言依存しているテストは無い。
- `src/rtsp/subscriber.rs::tests::depacketize_aac_with_multiple_aus` は `sizelength=13` / `indexlength=3` の正常系 (AU 2 個) を assert する。エラー文言は見ていないため統合後も pass する。
- 同 `depacketize_aac_rejects_zero_au_header_length` は `depacketize` 関数自身が返す `"invalid AAC RTP payload: AU header length must be greater than 0"` の Err 文言を assert する。BitReader 由来ではないため統合の影響を受けない。
- `grep -rn 'BitReader' src/ tests/ pbt/` の結果、`tests/` / `pbt/` 配下に `BitReader` 利用は無く、統合の波及は `src/rtsp/subscriber.rs` 1 ファイルのみ。

## 設計方針

`src/rtsp/subscriber.rs::BitReader` を削除し、`crate::video::bit_reader::BitReader` を import して利用する。

### import スタイル

`src/rtsp/subscriber.rs` 内では `use crate::video::bit_reader;` でモジュール単位インポートし、呼び出し側は `bit_reader::BitReader::new(au_headers)` と書く。

- `BitReader` 単独を rtsp のトップレベルにインポートすると由来が不明瞭になる
- `bit_reader::BitReader` という記述で「video モジュール配下の bit_reader 汎用実装」であることが視認できる
- 既存 H.264 / H.265 経路は同じ video モジュール内のため `use crate::video::{bit_reader::BitReader, ...}` で型のみ import しているが、別モジュールを跨ぐ rtsp 経路ではモジュール単位の方が適切

### モジュール docstring の更新

`src/video/bit_reader.rs` 冒頭の docstring は現状「H.264 / H.265 の SPS パーサで共有するビット単位読み出しユーティリティ」とあり、SPS パーサ限定の表現になっている。本 issue で MPEG4-GENERIC AAC AU header パースでも使うようになるため、docstring を「ITU-T 系仕様で標準的な MSB ファーストのビット単位読み出しを提供するユーティリティ。H.264 / H.265 SPS パーサや MPEG4-GENERIC AAC AU header パースで共有する。」のように更新する。モジュール自体の再配置 (`src/video/` 配下から外す) は本 issue では行わない (`## 残懸念` 参照)。

### API 差分の解消

#### 置換前後のスニペット

```rust
// 置換前 (subscriber.rs)
let mut bit_reader = BitReader::new(au_headers);
let size = bit_reader.read_bits(self.size_length)? as usize;
let _ = bit_reader.read_bits(index_bits)?;
```

```rust
// 置換後 (subscriber.rs)
let mut bit_reader = bit_reader::BitReader::new(au_headers);
let size = bit_reader
    .read_u(self.size_length as usize)
    .map_err(|e| e.with_context("invalid AAC AU header"))? as usize;
let _ = bit_reader
    .read_u(index_bits as usize)
    .map_err(|e| e.with_context("invalid AAC AU header"))?;
```

- `self.size_length` / `index_bits` (= `self.index_length` または `self.index_delta_length`) はいずれも `u8` のため `as usize` キャストを追加 (u8 → usize は無損失)。
- 戻り値の `as usize` (u32 → usize) は既存通り維持。
- Err は `Error::with_context` で AAC AU header 由来であることを示す文言を前置する (`### エラーメッセージの粒度` 参照)。`crate::Error` は `Display` を意図的に実装していない (`src/error.rs` 冒頭の docstring 参照) ため、`format!("{e}")` は使えない。`Error::with_context(&str)` は `reason` の前に `"{context}: {reason}"` を組み立て、`location` / `backtrace` を維持する公式 API なので、こちらを使う。
- 独自 BitReader の `impl` 内でしか参照していない `Error::new` は無いため、`use Error` を削る必要はない。

### bit_count == 0 経路の挙動同等性

旧 `BitReader::read_bits(0)` は `bit_count == 0` で早期 `Ok(0)` を返す。新 `BitReader::read_u(0)` は `n > 32` チェックを通過した後 (`0 > 32` は false) `for _ in 0..0 { ... }` がループ 0 回となり、初期値 `value = 0` をそのまま `Ok(0)` で返す。空バッファ (`BitReader::new(&[])`) でも両者とも `Ok(0)` を返す (新 BitReader は `read_bit` を呼ばないため `byte_pos >= data.len()` の Err 経路に入らない)。

- `self.size_length == 0` は `select_audio_track` の事前検証で Err 化されているため呼び出し経路に到達しない。
- `index_length == 0` / `index_delta_length == 0` は RFC 3640 §3.3.6 で許容され、`AacRtpDepacketizer::depacketize` ループは `consumed_bits += 0` でループが続く正常動作を期待する。新旧で挙動一致するため安全。

### bit_count > 32 経路の挙動差

旧 `BitReader::read_bits(bit_count: u8)` は `bit_count = 33..=255` でもループを回し、`value = (value << 1) | bit` の左シフトで上位ビットが落ちて壊れた `u32` を `Ok` で返す可能性があった (バッファ末尾を超えた場合のみ Err)。新 `BitReader::read_u(n: usize)` は `n > 32` で即 `Err("bit reader: read_u with n > 32 (n={n})")`。

`size_length` / `index_length` / `index_delta_length` は SDP fmtp で `u8::parse` 経由で受けるため理論上 33..=255 を取りうるが、RFC 3640 §3.3.6 上は `sizeLength` は AU-size フィールドのビット幅 (実用上 13)、`indexLength` / `indexDeltaLength` は AU-index / AU-index-delta のビット幅 (実用上 3)。32 超は実機 RTSP publisher で発生しない想定。

本 issue ではこの挙動差を「不正値を素通ししない方が堅牢」として受容し、`### RFC 3640 §3.3.6 準拠の fmtp パラメータ検証` で fmtp パース時点で明示的に Err 化することで `read_u` の Err 経路に到達する前段で fail-fast する。

### RFC 3640 §3.3.6 準拠の fmtp パラメータ検証

`select_audio_track` の fmtp パース直後 (現状 `size_length == 0` 検査の隣) に以下を追加する。

```rust
if size_length > 32 {
    return Err(Error::new("AAC fmtp sizeLength must be 32 or less"));
}
if index_length > 32 {
    return Err(Error::new("AAC fmtp indexLength must be 32 or less"));
}
if index_delta_length > 32 {
    return Err(Error::new("AAC fmtp indexDeltaLength must be 32 or less"));
}
```

これにより新 BitReader 経由の Err パスを通る前に session 確立時点で fail-fast する。

### エラーメッセージの粒度

新 `BitReader::read_u` のエラーメッセージ `"bit reader: exhausted before requested read"` は BitReader 単体の状態を述語にしており、H.264 / H.265 SPS パース経路と同じ文言になる。ログから AU header 経路の Err と SPS パース経路の Err を区別できるよう、`AacRtpDepacketizer::depacketize` 内で `Error::with_context("invalid AAC AU header")` を介して context を前置する。最終 reason は `"invalid AAC AU header: bit reader: exhausted before requested read"` のように元 reason を保ったまま prefix が付く形になる (`Error::with_context` の挙動は `src/error.rs::Error::with_context` 参照)。

H.264 / H.265 SPS パース経路は wrap しない (これらは video モジュール内で encoder / decoder / writer 経路から呼ばれ、文脈が呼び出し側で自明)。AAC AU header は rtsp モジュール固有なので wrap 側に揃える非対称は許容する。

### Exp-Golomb 系メソッドの余剰

`src/video/bit_reader.rs::BitReader` には `read_ue` / `read_se` / `skip_ue` / `skip_se` が pub メソッドとして実装されているが、AAC AU header パースでは `read_u` のみを使う。余剰 API が見えること自体は dead code ではなく lint も発生しないため受容する。

### 統合不可と判断した場合の代替

`subscriber` 側を温存しつつ struct 名を狭める (`AuHeaderBitReader` 等で `BitReader` 汎用名の二重確保を解消)。

統合不可と判断する条件 (いずれにも該当しない限り統合一択):

- 実装着手時に `grep '"bitstream is truncated"' src/ tests/ pbt/` でエラー文言を assert している箇所が新たに見つかる
- 新 BitReader の API で AAC AU header の挙動が等価に再現できない (現状の事前調査で `bit_count == 0` / `bit_count > 32` の差分まで確認済みのため発生しない想定)

## 推奨パッチ順序

変更範囲が極小のため 1 コミットで完結させる。各ステップ完了時点で `cargo check && cargo test` が pass すること。

1. `src/video/bit_reader.rs` 冒頭の docstring を MPEG4-GENERIC AAC AU header 用途を含む形に更新する (`### モジュール docstring の更新` 参照)。
2. `src/rtsp/subscriber.rs` の `struct BitReader` / `impl BitReader` (`#[derive(Debug)]` 行を含む) を削除する。
3. `use crate::video::bit_reader;` を `src/rtsp/subscriber.rs` の use 宣言に追加する。
4. `AacRtpDepacketizer::depacketize` 内の `BitReader::new` を `bit_reader::BitReader::new` に、`read_bits(x)` を `read_u(x as usize).map_err(|e| e.with_context("invalid AAC AU header"))?` に書き換える (`### 置換前後のスニペット` 参照)。
5. `select_audio_track` の fmtp パース直後に `size_length > 32` / `index_length > 32` / `index_delta_length > 32` の Err 化を追加する (`### RFC 3640 §3.3.6 準拠の fmtp パラメータ検証` 参照)。
6. `## テスト追加` の 5 テストを `src/rtsp/subscriber.rs::tests` に追加する。

コミットメッセージは `shiguredo-git` 規約の `{SEQ} {TITLE}` 形式 (`0058 RTSP AU header パース用の独自 BitReader を共有 BitReader に統合する`) とする。

## テスト追加

以下を `src/rtsp/subscriber.rs::tests` に新規追加する。

- `depacketize_aac_with_zero_index_length`: `index_length = 0` / `index_delta_length = 0` の fmtp で複数 AU を取り出せること (RFC 3640 §3.3.6 で許容される正常系を、新 BitReader の `read_u(0) -> Ok(0)` 経路として担保)。
- `select_audio_track_rejects_size_length_over_32`: fmtp に `sizelength=33` を含む SDP で session 確立時点で Err になること。文言は `"AAC fmtp sizeLength must be 32 or less"`。
- `select_audio_track_rejects_index_length_over_32`: 同上 (`indexlength=33`、文言は `"AAC fmtp indexLength must be 32 or less"`)。
- `select_audio_track_rejects_index_delta_length_over_32`: 同上 (`indexdeltalength=33`、文言は `"AAC fmtp indexDeltaLength must be 32 or less"`)。
- `depacketize_aac_wraps_bit_reader_error`: `au_headers` slice が消費しきれない (例: `au_headers_length_bits` を意図的に大きく設定して `read_u` が枯渇する) packet で、`depacketize` 由来の Err の `reason` が `"invalid AAC AU header: "` で始まることを assert (`with_context` の挙動担保。`crate::Error.reason` フィールドを直接参照するか、`err.display()` を使う)。

既存 `depacketize_aac_with_multiple_aus` / `depacketize_aac_rejects_zero_au_header_length` は変更不要 (本文 `### 確認済み事項 (grep / テスト)` 参照)。

## 完了条件

設計方針と完了条件を 1:1 対応で整理する。

### 削除・置換

- `src/rtsp/subscriber.rs::BitReader` 構造体と `impl` (`#[derive(Debug)]` 行を含む) が削除されている。
- `AacRtpDepacketizer::depacketize` 内の `BitReader::new(...)` / `read_bits(...)` が `bit_reader::BitReader::new(...)` / `read_u(... as usize).map_err(|e| e.with_context("invalid AAC AU header"))?` に置き換わっている。
- `use crate::video::bit_reader;` が `src/rtsp/subscriber.rs` の use 宣言に追加されている。
- `src/video/bit_reader.rs` 冒頭の docstring が MPEG4-GENERIC AAC AU header 用途を含む形に更新されている。

### fmtp 上限検査

- `select_audio_track` の fmtp パース直後に `size_length > 32` / `index_length > 32` / `index_delta_length > 32` の Err 化が追加されている。

### テスト

- `## テスト追加` の 5 テストが追加され pass する。
- 既存テスト `depacketize_aac_with_multiple_aus` / `depacketize_aac_rejects_zero_au_header_length` が pass する。

### CI / feature gate

本 issue の変更は `AacRtpDepacketizer::depacketize` / `select_audio_track` (feature 非依存) と video 配下の docstring のみを触るため、feature gate 別 build は不要。デフォルト build のみで足りる。

- デフォルト build: `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test && cargo fmt --all -- --check` がパスする。

### CHANGES.md

CHANGES.md への記載は **不要**。RTSP 機能自体が `## develop` 内の未リリース機能で、本 issue の変更 (BitReader 統合・fmtp 上限検査・エラー文言変化) はいずれも未リリース機能の中間修正に該当する。`shiguredo-changelog` 規約「変更履歴は派生元ブランチとの最終的な差分のみを記載すること。開発ブランチ内の中間状態の修正は記載しないこと」に従い、エントリは追加しない。closed/0048 がリリース済み機能 (video_toolbox H.265 / nvcodec H.265 が 2025.2.0 以降 release 済み) の hvcC 内部実値化を `[UPDATE]` で記載した判断軸とは異なる点に注意する。

## 関連

- closed/0048: `BitReader` を `src/video/bit_reader.rs` に新設し H.264 / H.265 SPS パーサで共有した先行 issue。本 issue はその `### 残懸念` で予告された `src/rtsp/subscriber.rs::BitReader` 統合を実施する。

## 残懸念

本 issue では扱わず、将来別 issue として起票候補:

- **`src/video/bit_reader.rs` のモジュール配置**: 現状 video モジュール配下にあるが、本 issue で rtsp モジュールからも使うようになり video 専用ではなくなる。`src/bit_reader.rs` (クレートルート直下) への格上げ余地があるが、import 文を 3 ファイル (h264.rs / h265.rs / subscriber.rs) 一括変更する範囲拡大になるため本 issue では行わない。
- **`bit_reader::BitReader` の可視性**: 現状 `pub` だが、crate 外から参照されていない。`pub(crate)` への引き下げ余地があるが、本 issue のスコープ外。

## 解決方法

推奨パッチ順序の 6 ステップを 1 コミットでまとめて対応した。

### 実装内容

1. **bit_reader.rs docstring 更新**: 用途を H.264 / H.265 SPS パーサ限定の記述から MPEG4-GENERIC AAC AU header パースを含む形に拡張した。Exp-Golomb 系メソッドが H.264 / H.265 経路のみで利用される旨も明記した。
2. **subscriber.rs の独自 BitReader 削除**: `struct BitReader` / `impl BitReader` (`#[derive(Debug)]` 行を含む) を削除した。
3. **import 追加**: `use crate::video::{..., bit_reader}` を `video::{VideoFormat, VideoFrame}` ブロックに追加し、`bit_reader::BitReader::new(...)` でモジュール単位で参照する形にした。
4. **`AacRtpDepacketizer::depacketize` の置換**: `read_bits(u8)` を `read_u(u8 as usize)` に書き換え、Err 経路は `Error::with_context("invalid AAC AU header")` で前置 context を付けた。ローカル変数名は `bit_reader` モジュール名と衝突しないよう `reader` にリネームした。
5. **fmtp 上限検査追加**: `select_audio_track` の `size_length == 0` 検査の隣に `size_length > 32` / `index_length > 32` / `index_delta_length > 32` の Err 化を追加し、RFC 3640 §3.3.6 値域外の SDP を session 確立時点で fail-fast するようにした。
6. **テスト追加**: `src/rtsp/subscriber.rs::tests` に新規 5 件 (`depacketize_aac_with_zero_index_length` / `depacketize_aac_wraps_bit_reader_error` / `select_audio_track_rejects_size_length_over_32` / `select_audio_track_rejects_index_length_over_32` / `select_audio_track_rejects_index_delta_length_over_32`) を追加し、audio fmtp で SDP を組み立てる helper (`build_test_sdp_with_audio_fmtp` / `parse_audio_track`) も追加した。

### CHANGES.md

RTSP 機能が未リリースのため、`shiguredo-changelog` 規約「開発ブランチ内の中間状態の修正は記載しないこと」に従い CHANGES.md エントリは追加しない。

### 検証

- `cargo check`: pass
- `cargo clippy --all-targets -- --deny warnings`: pass
- `cargo fmt --all -- --check`: pass
- `cargo test --lib 'rtsp::subscriber::tests'`: 31 件 pass (新規 5 件含む)
- `cargo test --lib 'video::bit_reader'`: 5 件 pass
- `cargo test --lib 'video::h264'`: 43 件 pass
- `cargo test --lib 'video::h265'`: 40 件 pass
