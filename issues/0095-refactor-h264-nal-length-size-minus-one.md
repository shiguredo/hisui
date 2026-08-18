# H.264 サンプルデータの NAL 区切りバイトサイズを lengthSizeMinusOne に基づいて汎用化する

- Created: 2026-08-04
- Completed: 2026-08-18
- Branch: feature/refactor-h264-nal-length-size-minus-one
- Polished: {YYYY-MM-DD}

## 目的

inspect コマンドの H.264 NAL 情報取得が「区切りバイトサイズ = 4 固定」を仮定しており、`lengthSizeMinusOne` が 3 以外の MP4 ファイルで正しくパースできない。AVCC の仕様に基づいて可変長の NAL 長フィールドに対応する。

## 現状

- `src/subcommand_inspect.rs` の `VideoCodecSpecificInfo::H264` 生成 (サンプルデータの NAL パース) が、NALU の区切りバイトサイズを 4 バイト固定と仮定している。この仮定は Sora 録画由来の名残であり、コメントも Sora 前提の表記だった (本 issue 起票時に一般化済み)
- 元々 `src/video/h264.rs` に存在した avcC の `length_size_minus_one` 検証 (`parse_avcc_sps_pps_lists`) は、録画機能 (WebM リーダー) の削除に伴い削除された (WebM CodecPrivate 専用パーサーで、 MP4 経路では `sample_entry` の `AvccBox` が使えるため再導入は不要)
- hisui の MP4 出力は `NALU_HEADER_LENGTH = 4` 固定 (`src/video/h264.rs` の `AvccBox` 構築) のため、自分の出力を読む分には問題ないが、inspect が読み込む外部 MP4 では `lengthSizeMinusOne` が 0〜2 (1〜3 バイト長) のファイルも存在し得る

## 設計方針

- AVCC ではサンプルデータの NAL 長フィールドは avcC ボックスの `lengthSizeMinusOne` (0〜3 = 1〜4 バイト) で決まる
- サンプルデータの NAL パース時に、対応トラックの avcC から取得した `lengthSizeMinusOne` を使って長フィールドを可変長で読み取る
- 変更対象は `src/subcommand_inspect.rs` の `VideoCodecSpecificInfo::new` (`VideoFormat::H264` 分岐) に閉じる

## 解決方法

- `VideoCodecSpecificInfo::new` の `VideoFormat::H264` 分岐で、NAL 長フィールドを読み取るバイト数 (`length_size`) を次から取得する
  - `sample.sample_entry` (`VideoFrame` の `pub` フィールド) → `SharedSampleEntry::get()` → `SampleEntry::Avc1` → `avcc_box.length_size_minus_one.get() + 1` (1〜4)
  - 取得パターンは既存の `src/rtmp/frame.rs` の `extract_nalu_length_size` を参考にする
- `sample.sample_entry` が `None`、または `SampleEntry::Avc1` でない場合は `NALU_HEADER_LENGTH` (4 バイト固定) にフォールバックする
  - `VideoFrame` の不変条件 (`docs/internals/sample_entry_invariant.md`) では圧縮フォーマットの `VideoFrame` は常に `Some` を持つため、フォールバックは防御的措置
- `VideoFormat::H264` 分岐の NAL パースを、`length_size` バイトの長フィールドを big-endian で読む可変長ロジックに変更する
  - 現状の「4 バイト固定」(`u32::from_be_bytes`) を廃止し、`length_size` 1〜4 に応じて読むバイト数を変える
  - 読み取りロジックは `subcommand_inspect.rs` 内のヘルパー関数として実装する (共通関数化はしない)
  - 符号化側 (`Annex B → AVC`) の `src/video/h264.rs` の `convert_annexb_to_nalu` は逆方向のため再利用しない
- 変更は `src/subcommand_inspect.rs` のみに閉じる

## 完了条件

- `lengthSizeMinusOne` が 3 以外の MP4 を入力に与えても、inspect が H.264 NAL 情報 (type / nri) を正しく出力できること
- `lengthSizeMinusOne` が 3 の既存 MP4 の読み込みに回帰がないこと
- `VideoCodecSpecificInfo::new` の単体テストで `length_size` 1〜4 の各ケース (NAL 長フィールドが正しく読めること) を検証する

## 実装時の決定事項 (2026-08-18)

- **可変長 NAL 長フィールドの実装**: `VideoFormat::H264` 分岐で `sample.sample_entry` → `SharedSampleEntry::get()` → `SampleEntry::Avc1` → `avcc_box.length_size_minus_one.get() + 1` から `length_size` (1〜4) を取得し、可変長の NAL 長フィールドを big-endian で読むヘルパー `read_nal_length` を新設した。`sample_entry` が `None` または非 `Avc1` の場合は `NALU_HEADER_LENGTH` (4 バイト固定) にフォールバックする
- **エラーパスのテスト追加**: 長さ 0 の NAL、長さプレフィックスが残データを超える不正入力を `VideoCodecSpecificInfo::new` に与えて `None` が返ることを検証するテストを追加した
- **多バイト長 NAL のテスト追加**: length_size ごとに「長フィールドの複数バイトにまたがる長さ」の NAL (length_size=1 では 255 境界、以降は 300 / 70000 / 200000) を含めて、encode → parse → type 抽出の往復を検証するテストを追加した
