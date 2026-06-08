# vmaf / tune サブコマンドを Sora 録画前提から汎用的に使えるようにする

- Priority: Medium
- Created: 2026-06-08
- Model: Opus 4.8
- Branch: feature/refactor-generic-vmaf-tune
- Reporter: @sile

## 目的

現状の `vmaf` / `tune` サブコマンドは Sora 録画ディレクトリ（`ROOT_DIR`）と
Sora 録画用のレイアウト（`recording_layout::Layout`）を前提とした実装になっており、
Sora 録画以外の任意の映像ソースに対して VMAF 評価やエンコードパラメータ調整を
行うことができない。

VMAF による品質評価やパラメータ探索という機能自体は、Sora 録画かどうかに
依存しない汎用的なものであり、任意の入力映像に対して使えるようにすることで
利用範囲を大きく広げられる。本 issue ではこの汎用化を行う。

## 優先度根拠

- ユーザー（@sile）からの「vmaf / tune は今は Sora 録画前提なので、もっと汎用的に
  使えるようにしたい」というフィードバックに基づくため、対応の必要性は明確である。
- 一方で、現状でも Sora 録画に対しては動作しており機能不全ではないこと、
  および後述の通り設計判断（汎用化の方式・コマンド体系）を伴うことから、
  最優先（High）ではなく Medium とする。

## 現状

- `vmaf` / `tune` サブコマンドは `src/sora/recording_subcommand_vmaf.rs` /
  `src/sora/recording_subcommand_tune.rs` に置かれており、モジュール構成上も
  Sora 録画機能（`src/sora/`）の一部となっている。
- 両サブコマンドとも引数として Sora 録画アーカイブを指す `ROOT_DIR` を必須で要求する。
  - `vmaf`: `ROOT_DIR` の example は `/path/to/archive/RECORDING_ID/`。
  - レイアウト内の相対パスの基点が `ROOT_DIR` となり、`ROOT_DIR` 外のファイル参照は
    エラーになる。
- レイアウトの組み立てに `recording_layout::Layout::from_layout_json_file_or_default()`
  を使っており、これは Sora 録画用レイアウトの抽象である。
- 映像ソースの読み込みに `recording_reader::VideoReader::from_source_info()` を用いている。
- なお、レイアウト JSON 自体は `video_sources` / `audio_sources` に任意の mp4 パスを
  列挙できる構造のため、レイアウトの仕組み自体はある程度汎用的である。
  Sora 録画前提となっている主因は次の 3 点と考えられる。
  1. サブコマンドが `src/sora/` 配下に置かれている（モジュール上の位置付け）。
  2. `ROOT_DIR`（Sora 録画アーカイブディレクトリ）を必須としている点。
  3. `recording_layout::Layout` という Sora 録画用レイアウトに直接依存している点。

## 設計方針

以下は方針の候補であり、着手前に設計判断が必要（詳細は「未確定事項」参照）。

- `vmaf` / `tune` を Sora 録画専用ではなく、任意の映像入力に対して動作する
  汎用サブコマンドとして再構成する。
  - 例えば、入力映像を直接指定できるようにする、あるいは `ROOT_DIR` を必須ではなく
    任意の基準ディレクトリ（レイアウト内相対パスの基点）として扱えるようにするなど。
- Sora 録画固有の依存（`recording_layout` / `recording_reader` 等）を、
  汎用的な合成・読み込み経路から利用できる形に切り出す、もしくは抽象化する。
  - `compose` サブコマンドも同じ `recording_layout::Layout` と `ROOT_DIR` を
    共有しているため、汎用化の単位（vmaf / tune だけか、compose も含めた共通基盤か）を
    検討する。
- モジュール配置を見直し、汎用機能であることがコード構成からも分かるようにする
  （`src/sora/` 配下からの移動を検討）。

## 完了条件

- Sora 録画ディレクトリを用意しなくても、任意の入力映像に対して `vmaf` /
  `tune` を実行できること。
- 既存の Sora 録画に対する `vmaf` / `tune` の利用方法が引き続き動作すること
  （後方互換を壊す場合は `[CHANGE]` として明記し、移行方法を示すこと）。
- 汎用化された経路に対するテスト（PBT / 単体テスト）が追加されていること。

## 解決方法

未確定事項を確定させた上で具体化する。現時点では以下を想定する。

- `vmaf` / `tune` の引数体系を汎用入力に対応した形へ変更する。
- Sora 録画固有のレイアウト・リーダー依存を汎用経路から切り離す。
- モジュール配置を見直す。
- 汎用経路のテストを追加する。

## 未確定事項

本 issue は設計判断を伴うため、着手前に以下を確定させること
（必要に応じて `issues/pending/` への移動を検討する）。

- 汎用化の対象範囲（`vmaf` / `tune` のみか、`compose` を含む共通基盤の汎用化か）。
- 入力の指定方法（個別の映像ファイル指定か、`ROOT_DIR` を任意基準ディレクトリへ
  一般化するか、両対応か）。
- 後方互換の扱い（既存の Sora 録画向け呼び出しを維持するか、`[CHANGE]` とするか）。
- コマンド体系・モジュール配置の最終形。
