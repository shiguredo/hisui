# 各エンコーダーで指定可能なパラメーターを最新に追従する

- Priority: Medium
- Created: 2026-05-29
- Completed:
- Model: Opus 4.7
- Branch: feature/add-latest-encoder-params
- Polished: 2026-05-29

## 目的

hisui は各種ビデオエンコーダー (libvpx VP8/VP9, openh264, svt-av1, VideoToolbox H.264/H.265, NVENC H.264/H.265/AV1) のパラメーターを `EncodeConfig` 経由で公開している。これらは依存している C ライブラリ (libvpx, openh264, svt-av1 等) のバージョン更新によって指定可能な項目が増減するが、現状の hisui 側の公開・パース部分は **互換性重視で古い項目セットのまま** になっている部分が多い。`shiguredo_*` 系 crate の `EncoderConfig` が rate 系を外出しするなどインターフェースを整理した変更にも、hisui 側が全て追従できていない。

性能・画質はエンコーダーパラメーターのチューニング余地が大きく、基本的に **常に最新のパラメーターセットに追従** するのが望ましい。本対応では、各エンコーダーで指定可能なパラメーターを最新に揃え、追加されたパラメーターは公開、削除・廃止されたパラメーターは **警告ログを出した上で無視** する方針で整理する。

## 優先度根拠

- 性能や画質に直結するため、最新パラメーターへの追従はユーザー価値が大きい。
- 一方、現状のままでも合成は動作するため High ではない。
- C ライブラリのバージョン更新は継続的に発生し、追従が遅れるほど差分が大きくなって対応コストが累積する。早めに「最新追従するメカニズム」を整えておく価値がある。
- 以上から Medium 妥当。

## 現状

### `EncodeConfig` の構成

- `src/encoder.rs:335-350` (`EncodeConfig`)
  - 各エンコーダーの設定型を、それぞれの crate (`shiguredo_libvpx::EncoderConfig`, `shiguredo_openh264::EncoderConfig`, `shiguredo_svt_av1::EncoderConfig`, `shiguredo_video_toolbox::EncoderConfig`, `shiguredo_nvcodec::EncoderConfig`) からそのまま流用してフィールドに持っている。
- `src/encoder.rs:353-361` (`VideoEncoderOptions`)
  - `bitrate: usize`, `frame_rate: FrameRate` が **構造体の外側に直接生えている**。`encode_params: EncodeConfig` とは独立。
  - 各エンコーダー実装 (例: `src/encoder/openh264.rs:21-27`) では `..options.encode_params.openh264.clone()` でベース設定を取り、`options.bitrate` で `target_bitrate` を上書きする構造。
  - これが「rate 外出し」の現状実装。`EncodeConfig` 側にも `target_bitrate` フィールドは残っており、最終的にどちらが効くかが分かりにくい。

### JSON 経由のパラメーター指定

- `src/sora/recording_layout_encode_params.rs:15-67`
  - layout.json の `libvpx_vp8_encode_params`, `libvpx_vp9_encode_params`, `openh264_encode_params`, `svt_av1_encode_params`, `video_toolbox_h{264,265}_encode_params`, `nvcodec_{h264,h265,av1}_encode_params` キーから読む。
  - **未知のキー (タイポや古い名前) は `_ => {}` で無視される** (62 行目)。エラーにも警告にもならない。
- 各エンコーダーパラメーター JSON パーサー (`src/sora/recording_encoder_*_params.rs`, 計 5 ファイル / 約 1000 行)
  - 例: `recording_encoder_libvpx_params.rs:60` のコメントに `// - target_bitrate` (= rate 系は外出しなので読まない)
  - `to_member("min_quantizer").optional()` のように **明示的に列挙したキーのみ** パース。`shiguredo_libvpx::EncoderConfig` の最新フィールド一覧と乖離があれば、指定しても効かない。
- `recording_layout_encode_params.rs:70-135` (`new_config_from_default_layout`)
  - `DEFAULT_LAYOUT_JSON` (おそらく `layout-examples/default.jsonc` 由来) を起動時にパースして既定値とする。デフォルト値の更新もここに乗ってくる。

### 警告の現状

- 未知のキー: 全エンコーダーで一律 silent ignore (`_ => {}` 等)。
- 廃止されたキー: hisui 側に古いマッピングが残っていれば「指定されたが C ライブラリ側で意味を持たない」ケースが発生しうる。現状はパースは通り、`EncoderConfig` に値はセットされ、エンコーダー初期化時に無視されるか、最悪 panic / エラー。

### 利用箇所

- `EncodeConfig` のユーザーは以下:
  - 録画合成 (`src/sora/recording_*` 経由)
  - obsws の HLS / MPEG-DASH 出力 (`src/obsws/coordinator/output_dash.rs:842`, `output_hls.rs:859`、`encode_config_with_keyframe_interval` 経由)
  - obsws のその他出力 (録画・ストリーミング)
- いずれも `LayoutEncodeParams::default()` をベースに、必要なら JSON で上書きするフロー。

## 設計方針

### 基本方針

1. **追加されたパラメーターは公開する**: 依存 crate (`shiguredo_libvpx` 等) で新規に追加されているフィールドを、hisui の JSON パーサー (`recording_encoder_*_params.rs`) と既定 layout に反映する。
2. **rate 系の二重管理を整理する**: `VideoEncoderOptions.bitrate` で上書きする現行ロジックは維持しつつ、`EncodeConfig.<codec>.target_bitrate` 側は JSON でも指定不可とし、コメントで「rate 系は VideoEncoderOptions 経由のみ」と明示する。
3. **古いキー / 廃止されたキーは警告して無視**: パーサーで未知のキー (= もはや存在しないキー、または typo) を見つけたら `tracing::warn!` で 1 回ログを出して無視する。プログラムは止めない。
   - 現状の silent ignore は「typo に気付けない」「古いキーを指定しているのに動いていると勘違いする」原因になる。**警告化は本対応の主要価値の 1 つ**。
4. **既定値も最新に揃える**: `DEFAULT_LAYOUT_JSON` の中身を依存 crate の最新推奨値に合わせて更新する。後方互換のためにユーザー指定の上書きは引き続き可能。

### パラメーター追従の判断基準

| 種別 | 例 | 対応 |
| ---- | -- | ---- |
| 追加された新規パラメーター | libvpx の新 `cpu_used` 値、svt-av1 の `--preset` 拡張等 | hisui の JSON パーサーに追加、`recording_layout_encode_params.rs` の match に追加 |
| 削除/廃止されたパラメーター (依存 crate 側で消えた) | 古い `static_threshold` など | hisui 側の対応 case を削除し、`_ => warn!` 経路で警告される状態にする |
| 名称変更されたパラメーター | (例: `target_bitrate` → `target_bit_rate` のような変更) | 新名で公開しつつ、旧名の case を残して `warn!("renamed to new_name")` して新名にマッピング |
| rate 系 (bitrate, framerate) | `target_bitrate`, `frame_rate_numerator` 等 | JSON では受け取らず、`VideoEncoderOptions` 経由のみ |

### 警告の実装

```rust
for (key, value) in obj {
    match &*key.to_unquoted_string_str()? {
        "min_quantizer" => { ... }
        "max_quantizer" => { ... }
        // ...既知のキー...
        unknown => {
            tracing::warn!(
                "ignored unknown libvpx_vp8 encode param: {unknown}"
            );
        }
    }
}
```

- 警告ログは英語 (CLAUDE.md 準拠)。
- 警告対象はパース処理のレベルではなく **JSON の各オブジェクト単位** で出す（タイポを 1 つずつ警告する）。
- 同じキーが複数回現れた場合は最後の値を採用する (現状の挙動を維持)。

### rate 外出しに伴うインターフェース整理

- `VideoEncoderOptions.bitrate` を持ち続けるが、各エンコーダー実装内で `EncodeConfig.<codec>.target_bitrate` の **値を読まないことを明示** する。
- 現状は `..options.encode_params.openh264.clone()` のように spread しているため、`target_bitrate` が `encode_params` 側にも残っていると JSON 経由で上書きされうる。`encode_params` 側の `target_bitrate` 等の rate 系は **構造体上は存在するが、明示的に default 値で潰す** か、もしくは JSON パーサー側で無視する。
- `recording_encoder_*_params.rs` のコメント (例: `// - target_bitrate`) は既に「JSON では読まない」を意図しているように見えるが、`EncodeConfig` 経由で他経路 (server RPC 等) から渡されたときも一貫して効かないようにする。

### 依存 crate のバージョン管理

- `Cargo.toml` の `workspace.dependencies` で固定している `shiguredo_libvpx`, `shiguredo_openh264`, `shiguredo_svt_av1`, 等のバージョンを **本対応と同 PR で最新に上げる**。これによって `EncoderConfig` の最新フィールド一覧が hisui に取り込まれる。
- バージョンアップに伴うコンパイルエラー (フィールド追加・削除) を hisui 側で吸収する。

## 完了条件

- `Cargo.toml` の `shiguredo_libvpx`, `shiguredo_openh264`, `shiguredo_svt_av1`, `shiguredo_video_toolbox`, `shiguredo_nvcodec` (feature 有効時) が最新版に追従していること。
- 各 `recording_encoder_*_params.rs` の JSON パーサーが、対応する `EncoderConfig` の最新フィールドを網羅していること (rate 系を除く)。
- 未知のキーが指定された際、`tracing::warn!` で警告ログが出ること。プログラムは継続すること。
- `DEFAULT_LAYOUT_JSON` (おそらく `layout-examples/default.jsonc`) の既定値が最新の推奨値に揃っていること。
- `cargo test` がすべて成功すること。
- CHANGES.md の `## develop` に以下を追記:
  - `[ADD] <エンコーダー名> の <パラメーター名> を指定できるようにする` (各エンコーダーごと)
  - `[ADD] エンコーダーパラメーターで未知のキーが指定された際に警告ログを出すようにする`
  - 必要なら `[CHANGE] <エンコーダー名> の <旧パラメーター名> を廃止する` (依存 crate 側で削除されたもの)
- 既定値変更によって出力されるメディアの品質が変わる可能性があるため、サンプルファイルでの目視確認 (1 ファイル/コーデックの規模で十分) を行ったログを残す。

## 解決方法

### 実装ステップ

1. **依存 crate の最新版を確認する**:
   - `shiguredo_libvpx`, `shiguredo_openh264`, `shiguredo_svt_av1`, `shiguredo_video_toolbox`, `shiguredo_nvcodec` の各 `EncoderConfig` の最新フィールド一覧を取得する (リポジトリの該当バージョンの src を grep するか、docs.rs で確認)。
2. **`Cargo.toml` のバージョンを最新に更新する**:
   - workspace 全体でビルドが通ることを確認する。フィールド名の変更などで hisui 側の差分が必要なら追従修正する。
3. **各 `recording_encoder_*_params.rs` を更新する**:
   - 新規追加されたフィールドを `to_member("...")` で読み出すケースを追加。
   - 廃止されたフィールド名のマッチ case を削除。
   - 未知のキーへの警告ログを末尾の `_` arm に追加。
4. **`recording_layout_encode_params.rs` の match を更新する**:
   - 新規エンコーダーが追加されていれば case を追加。
   - 未知のキーへの警告は同じ方針で。
5. **`DEFAULT_LAYOUT_JSON` の既定値を最新版の推奨値に揃える** (`layout-examples/default.jsonc` 等。実体パスは要確認)。
6. **rate 系の整理**:
   - `VideoEncoderOptions.bitrate` を最終的な真値とし、`EncodeConfig.<codec>.target_bitrate` 等が JSON 経由で誤って効かないことを担保する (パーサー側で無視 + コメント明示)。
   - 各 `encoder/<codec>.rs` の spread 順序を見直す（`..options.encode_params.<codec>.clone()` を取り込んだ後で必ず rate 系を上書きする現状の順序を維持しつつ、コメントで根拠を残す）。
7. **テスト追加**:
   - 各パーサーに対して、未知のキーを含む JSON を渡したときに警告が出る (+ パース結果が既知キーで埋まる) ことを検証する単体テストを追加。
   - 警告は `tracing` 経由なので、テストでは `tracing-subscriber` の test layer か、もしくは「パースが成功する」ことだけ確認する形に絞る (CLAUDE.md ルールでモック不可なため)。

### リスク・留意点

- 既定値の更新は **出力メディアの内容を変える** ため、後方互換的に注意が必要。最低限、出力ファイルが破損しないこと・主要コーデックで再生できることを目視確認する。
- 依存 crate のバージョン更新で **コンパイルが通らなくなる** 場合がある。その場合は本 issue の範囲を超える対応 (例: 新フィールドの必須化への対応) が必要になるかもしれず、その時点で別 issue として切り出すかを判断する。
- VideoToolbox / NVENC 等プラットフォーム固有のエンコーダーは、対応 OS / GPU でしか検証できない。検証可能な環境ごとに動作確認のログを残す。
- 「未知のキーは警告」方針は、ユーザーがタイポを直し忘れていた時に挙動が変わって見える原因にもなる。警告ログは英語で「unknown key '...' is ignored」のような形式で出し、grep しやすくする。
- 廃止されたパラメーターを別名で復活させたいケース (互換性のために旧名でも受けたい等) は今回はやらず、必要が出てから個別に追加する。

### 将来の継続運用

- 依存 crate のバージョンを上げる際は **同 PR で `recording_encoder_*_params.rs` の追従** を必ず実施する運用にする。
- 本対応で導入する「未知キー警告」は、運用ログから「ユーザーが指定し続けている古いキー」を観測する手段にもなる。観測結果を見て、必要なら互換マッピング (旧名 → 新名) を追加する判断材料にできる。
