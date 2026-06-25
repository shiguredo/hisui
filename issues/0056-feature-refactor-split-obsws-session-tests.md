# obsws session の単体テストファイルを機能群ごとに分割する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-split-obsws-session-tests
- Polished: 2026-06-25

## 目的

`src/obsws/session/tests.rs` が単一ファイルで巨大化しているため、機能群ごとにサブモジュールへ分割し、ファイル単体の規模を 1500 行以下に抑える。テスト関数本体・テスト関数名・テスト件数・ヘルパー本体・ヘルパー名は一切変更せず、ファイル間移動のみで構成変更する純粋なリファクタとする。

## 優先度根拠

Low。挙動・公開 API・テスト件数・テスト関数名・JSON 出力・state file の永続化フォーマットは一切変化しない内部リファクタで即時の機能影響はない。一方で、新しい obsws リクエスト追加時にテストの居場所が不明確化する継続的な開発生産性の負担がある (broken windows 原則)。CI のコンパイル時間そのものは Rust の test crate 単位が変わらないため大きく改善しない見込みであり、根拠から外す。

## 前提条件

本 issue は以下のいずれの open issue とも `src/obsws/session/tests.rs` 内で直接の行衝突は起きない (本 issue はテストファイル分割のみ、open 0046/0052 は本体側の変更のため)。ただし `tests/text_overlay.rs` / `tests/stream_service.rs` 内で参照するシンボル名や signature が 0046/0052 のマージ後に変わる可能性があるため、これらが merge 済みの状態で本 issue に着手することを推奨する:

- open issue 0046 (`feature/refactor-clarify-processor-validation-boundary`): `src/obsws/coordinator/output_stream.rs:359` 周辺の `start_stream_processors` 整理。`tests/output_stream.rs` 内のシンボル参照が間接的に影響する可能性
- open issue 0052 (`feature/refactor-obsws-parse-helpers`): `src/obsws/coordinator/text_overlay.rs` 周辺。`tests/text_overlay.rs` 内のシンボル参照が間接的に影響する可能性

並行作業する場合は最後にマージする側が機械的なコンフリクト (主に `use` 文) を吸収する。

## 現状

### 対象ファイルと規模 (2026-06-25 時点、`wc -l` 計測)

`src/obsws/session/tests.rs` 4739 行。テスト関数 99 件 (`#[tokio::test]`) + 共通ヘルパー 23 件 + text_overlay 専用ヘルパー 3 件。

比較:
- `src/obsws/state_file.rs` 2897 行 (本体側の次点)
- `src/obsws/state/tests.rs` 1361 行 / `src/obsws/response/tests.rs` 1389 行 (同じ `#[path]` パターンの先例、いずれも未分割で運用中)

### 既存の `#[path]` パターン (3 箇所)

- `src/obsws/session.rs:12-14`: 3 行構造で `#[cfg(test)]` / `#[path = "session/tests.rs"]` / `mod tests;`
- `src/obsws/state.rs:7-9`: 同じ 3 行構造で `state/tests.rs` を指す
- `src/obsws/response.rs:1322-1324`: 同じ 3 行構造で `response/tests.rs` を指す

いずれも `mod.rs` を採用せず、親 (`<module>.rs`) から `#[path]` 属性で `<module>/tests.rs` を 1 段ぶら下げる構成。**子ファイル (`<module>/tests.rs`) 内で更に `mod` 宣言を入れている前例は `src/` 配下に存在しない** (`src/obsws/state/tests.rs` および `src/obsws/response/tests.rs` を grep して `mod` 宣言ゼロを確認済み。本 issue で新規導入する構成)。

### 共通ヘルパーの所在 (実数 26 件、line 17-438 にヘルパー間にテスト 2 件が挟まる構造)

`tests.rs` の line 17-438 はヘルパー定義の塊だが連続ではなく、line 188 と line 236 に scene 系テスト 2 件が紛れ込んでいる。物理レイアウトは以下:

- line 17-185 (10 件、coordinator 構築系): `test_program_output` / `create_coordinator_handle` / `create_coordinator_handle_with_player_channels` (`#[cfg(feature = "player")]`) / `default_coordinator_handle` / `test_player_command_tx` (`cfg(feature = "player")`) / `test_player_media_tx` (`cfg(feature = "player")`) / `create_coordinator_handle_with_pipeline` / `create_coordinator_handle_with_pipeline_and_record_dir` / `create_initialized_coordinator_handle_with_pipeline` / `create_initialized_coordinator_handle_with_pipeline_and_record_dir`
- **line 188 と line 236 は Scene 系テスト 2 件** (`remove_current_scene_updates_program_output_state_without_pipeline` / `stale_scene_uuid_differs_from_current_program_scene_uuid`、移動先は `tests/scene.rs`)
- line 255-389 (10 件、JSON パーサ + `SessionAction` 解体): `parse_request_status` / `parse_request_type` / `parse_output_active` / `parse_response_scene_item_id` / `parse_identified_message` / `parse_event_type_and_intent` / `parse_request_batch_results` / `unwrap_send_text` / `unwrap_send_texts` / `unwrap_close`
- line 392-438 (3 件、セッション初期化): `identify_session` / `create_output` / `wait_for_processor_presence`

汎用ヘルパー計 23 件。

text_overlay 専用ヘルパー 3 件 (line 3949-4054 の text_overlay セクション内):

- `create_initialized_coordinator_with_text_overlay` (line 3959)
- `process_text_overlay_request` (line 4023)
- `parse_text_overlays_count` (line 4044、命名は JSON パーサ系だが利用箇所が text_overlay 関連 1 件のみのため text_overlay 専用扱い)

### テスト機能群と件数 (実測、合計 99 件)

| 機能群 | 件数 | 概略行数 |
|---|---|---|
| Lifecycle (identify / sleep / broadcast / authentication / RPC version) | 14 | 約 250 |
| Scene | 7 | 約 230 |
| Input | 6 | 約 230 |
| SceneItem | 9 | 約 505 |
| Output (record 系) | 6 | 約 500 |
| Output (stream 系) | 3 | 約 230 |
| Output (HLS / DASH 系) | 2 | 約 280 |
| Output (toggle / stop / start / その他 lifecycle) | 7 | 約 220 |
| Output (player 系、`#[cfg(feature = "player")]`) | 4 | 約 320 |
| Output (create / settings) | 17 | 約 800 |
| RequestBatch | 2 | 約 50 |
| PersistentData | 4 | 約 140 |
| StreamServiceSettings | 2 | 約 130 |
| TextOverlay (専用ヘルパー 3 件込み) | 16 | 約 650 |
| **合計** | **99** | **約 4530** |

サブモジュール想定行数の合計に共通ヘルパー (`common.rs` 約 430 行) と各サブモジュール固有の `use` 文・ファイル冒頭コメント (合計約 150 行) を足すと約 5110 行になり、現状 `tests.rs` 4739 行を超える。差分 (約 370 行) は分割により導入される `use` 宣言の重複と、機能群間の境界に置くセクションコメント・空行のオーバーヘッドに相当する。想定行数はあくまで概算で、実装後に再計測して完了条件 (1500 行以下) との整合を確認する。

### Output (record / stream / HLS / DASH) の物理的交錯

実コードでは record 系 / stream 系 / HLS / DASH のテストが行番号順に交錯している:

```
1593/1615/1698/1771/1851 (record) → 1916 (stream) → 1986 (record) →
2025 (stream) → 2109 (HLS) → 2239 (DASH) → 2392 (stream)
```

Phase 6-8 ではこの並びを「機能群名で抜き出して移動」するため、`git mv` のような単純なファイル間移動ではなく「ファイル内の関数ブロックを抜き出して新ファイルへ追加」になる。実装者はこの順序逆転を見落とすと、Phase 6 (record 系) で 1916 や 1986 を取り違えるリスクがあるため、関数名を見て該当群のみを抜き出すこと。

### 外部からの参照 (分割影響範囲)

`src/obsws/session/tests.rs` のパス文字列をリポジトリ全体で grep した結果、open な参照は 1 箇所のみ:

- `pbt/tests/prop_text_overlay.rs:10` の doc コメントが `src/obsws/session/tests.rs` を参照

`issues/closed/` 配下にも複数の行番号参照があるが、closed issue は historic record として更新しない方針とする (詳細は「対象外スコープ」)。

## 設計方針

### 1. ディレクトリ配置とエントリポイント

`src/obsws/session/tests.rs` を以下のディレクトリ構造に置き換える。`mod.rs` は採用せず (`coordinator.rs` + `coordinator/`、`session.rs` + `session/` と同じスタイル)、エントリポイント `tests.rs` から各サブモジュールを宣言する。

```
src/obsws/session/
├── output.rs                 (既存、本体側サブモジュール。変更なし)
└── tests/                    (新設)
    ├── common.rs             (共通ヘルパー集約)
    ├── lifecycle.rs
    ├── scene.rs
    ├── scene_item.rs
    ├── input.rs
    ├── output_record.rs
    ├── output_stream.rs
    ├── output_hls_dash.rs
    ├── output_misc_lifecycle.rs
    ├── output_player.rs
    ├── output_create.rs
    ├── request_batch.rs
    ├── persistent_data.rs
    ├── stream_service.rs
    └── text_overlay.rs
src/obsws/session/tests.rs    (エントリポイント、mod 宣言のみ)
```

合計 14 サブモジュール + `common.rs` の 15 ファイル。最大ファイル (`output_create.rs` 約 800 行) でも 1500 行マージンを 47% 確保する。

### 2. `#[path]` 属性によるサブモジュール解決方式

現状 `session.rs:13` の `#[cfg(test)] #[path = "session/tests.rs"] mod tests;` はそのまま維持する。エントリポイント `tests.rs` 内で各サブモジュールに `#[path]` 属性を明示する形を採用する。

```rust
// src/obsws/session/tests.rs (エントリポイント、目標 30 行前後)
#[path = "tests/common.rs"]
mod common;
#[path = "tests/lifecycle.rs"]
mod lifecycle;
#[path = "tests/scene.rs"]
mod scene;
#[path = "tests/scene_item.rs"]
mod scene_item;
#[path = "tests/input.rs"]
mod input;
#[path = "tests/output_record.rs"]
mod output_record;
#[path = "tests/output_stream.rs"]
mod output_stream;
#[path = "tests/output_hls_dash.rs"]
mod output_hls_dash;
#[path = "tests/output_misc_lifecycle.rs"]
mod output_misc_lifecycle;
#[path = "tests/output_player.rs"]
mod output_player;
#[path = "tests/output_create.rs"]
mod output_create;
#[path = "tests/request_batch.rs"]
mod request_batch;
#[path = "tests/persistent_data.rs"]
mod persistent_data;
#[path = "tests/stream_service.rs"]
mod stream_service;
#[path = "tests/text_overlay.rs"]
mod text_overlay;
```

採用根拠: Rust リファレンス (`https://doc.rust-lang.org/reference/items/modules.html#path-attribute-1`) によれば、`#[path]` で読み込まれたファイル内の子モジュールは「指定された実ファイルが置かれているディレクトリ」を基準に解決される。本構成では、明示的に各 `mod` に `#[path = "tests/<name>.rs"]` を付けることで、仕様解釈に依存しない一意な解決を保証する。`src/` 配下に本構成の前例はないため、**着手前の最小再現検証** を実装フェーズの最初に行う (完了条件参照)。

別案 (`session.rs` 側を `#[path = "session/tests/<name>.rs"]` 形式に書き換える / `#[path]` を外して通常規約に戻す) は採用しない。前者は `session/tests.rs` を維持する設計と矛盾、後者は他 2 ファイル (`state.rs` / `response.rs`) の `#[path]` パターンと整合しない。

### 3. サブモジュール構成と振り分け (99 件すべての完全リスト)

各テスト関数の振り分けを以下に確定する。テスト名・引数・属性 (`#[tokio::test]` / `#[cfg(feature = "player")]`) は変更しない。行番号は 2026-06-25 時点の `src/obsws/session/tests.rs` のもの。

#### `tests/common.rs` (ヘルパー 23 件、約 430 行)

「現状: 共通ヘルパーの所在」節に列挙した 23 件。`#[cfg(feature = "player")]` 付きヘルパー 3 件 (`create_coordinator_handle_with_player_channels` / `test_player_command_tx` / `test_player_media_tx`) を含む。

#### `tests/lifecycle.rs` (14 件、約 250 行)

| 行 | 関数名 |
|---|---|
| 441 | `on_connected_returns_hello_message_action` |
| 452 | `on_request_before_identify_returns_close_action` |
| 467 | `broadcast_custom_event_returns_event_when_general_subscription_enabled` |
| 501 | `sleep_request_returns_success_response` |
| 525 | `sleep_request_rejects_too_large_sleep_millis` |
| 549 | `duplicate_identify_returns_already_identified_close` |
| 566 | `reidentify_before_identify_returns_not_identified_close` |
| 578 | `reidentify_after_identify_returns_identified_message` |
| 600 | `identify_without_event_subscriptions_defaults_to_all` |
| 611 | `identify_with_event_subscriptions_updates_session_state` |
| 622 | `reidentify_updates_event_subscriptions_when_specified` |
| 640 | `reidentify_without_event_subscriptions_keeps_previous_value` |
| 1560 | `unsupported_rpc_version_returns_close_action` |
| 1572 | `invalid_authentication_returns_close_action` |

`sleep_*` 2 件は OBS-WebSocket の Sleep request 単体テストで、RequestBatch 内利用にも関係するが、独立した RPC タイプなので lifecycle に同居させる (別群を増やさない選択)。

#### `tests/scene.rs` (7 件、約 230 行)

| 行 | 関数名 |
|---|---|
| 188 | `remove_current_scene_updates_program_output_state_without_pipeline` |
| 236 | `stale_scene_uuid_differs_from_current_program_scene_uuid` |
| 658 | `create_scene_with_scene_subscription_returns_scene_created_event` |
| 683 | `set_current_program_scene_to_same_scene_returns_response_only` |
| 704 | `set_current_preview_scene_with_scene_subscription_returns_preview_event` |
| 738 | `set_current_preview_scene_to_same_scene_returns_response_only` |
| 759 | `remove_current_scene_with_scene_subscription_sends_scene_program_and_preview_events` |

line 188 / 236 はヘルパー間に挟まっている scene 系テスト。`common.rs` に持って行かないこと。

#### `tests/input.rs` (6 件、約 230 行)

| 行 | 関数名 |
|---|---|
| 823 | `create_and_remove_input_with_input_subscription_send_input_events` |
| 863 | `set_input_settings_with_input_subscription_sends_event` |
| 902 | `set_input_settings_with_input_subscription_does_not_send_event_on_error` |
| 940 | `set_input_name_with_input_subscription_sends_event` |
| 979 | `set_input_name_with_input_subscription_does_not_send_event_on_error` |
| 1030 | `set_input_name_with_invalid_input_uuid_type_returns_parse_error` |

#### `tests/scene_item.rs` (9 件、約 505 行)

| 行 | 関数名 |
|---|---|
| 1055 | `set_scene_item_enabled_with_scene_subscription_sends_event_when_changed` |
| 1118 | `set_scene_item_enabled_with_same_value_returns_response_only` |
| 1168 | `set_scene_item_locked_with_scene_subscription_sends_event_when_changed` |
| 1222 | `set_scene_item_transform_with_scene_subscription_sends_event_when_changed` |
| 1277 | `create_scene_item_with_scene_subscription_sends_created_event` |
| 1322 | `remove_scene_item_with_scene_subscription_sends_removed_and_reindexed_events` |
| 1396 | `remove_scene_item_tail_with_scene_subscription_does_not_send_reindexed_event` |
| 1466 | `set_scene_item_index_with_scene_subscription_sends_reindexed_event` |
| 1536 | `set_scene_item_enabled_missing_field_returns_missing_request_field_error` |

#### `tests/output_record.rs` (6 件、約 500 行)

| 行 | 関数名 |
|---|---|
| 1593 | `stop_record_when_inactive_returns_error_response` |
| 1615 | `start_record_with_mp4_file_source_can_start_and_stop` |
| 1698 | `start_record_with_mp4_file_source_can_stop_immediately_after_start` |
| 1771 | `start_record_with_multiple_audio_inputs_uses_audio_mixer` |
| 1851 | `start_record_with_no_inputs_succeeds` |
| 1986 | `start_record_with_multiple_video_inputs_builds_plan_successfully` |

#### `tests/output_stream.rs` (3 件、約 230 行)

| 行 | 関数名 |
|---|---|
| 1916 | `start_stream_with_no_inputs_succeeds` |
| 2025 | `start_stream_with_multiple_audio_inputs_uses_audio_mixer` |
| 2392 | `start_stream_with_multiple_video_inputs_builds_plan_successfully` |

#### `tests/output_hls_dash.rs` (2 件、約 280 行)

| 行 | 関数名 |
|---|---|
| 2109 | `hls_output_uses_program_mixers_after_scene_item_change` |
| 2239 | `dash_output_uses_program_mixers_after_scene_change` |

名前に `scene_item` / `scene` を含むが、テストの主目的は HLS/DASH 出力経路の検証なので `output_hls_dash.rs` に置く (prefix 衝突の解決方針: 出力フォーマット名を優先する)。

#### `tests/output_misc_lifecycle.rs` (7 件、約 220 行)

| 行 | 関数名 |
|---|---|
| 2448 | `toggle_stream_without_image_input_returns_toggle_request_type_error` |
| 2471 | `start_output_with_unknown_name_returns_not_found` |
| 2497 | `toggle_output_without_image_input_returns_toggle_request_type_error` |
| 2523 | `stop_output_when_record_is_inactive_returns_output_request_type_error` |
| 3178 | `hisui_remove_output_running_returns_error` |
| 3854 | `set_stream_service_settings_after_remove_returns_not_found` |
| 3895 | `start_output_uses_output_kind_even_when_name_matches_legacy_builtin` |

「停止 / 切替 / 削除 / 起動制御」に関する lifecycle テストで、Record/Stream/HLS/DASH/Player のいずれにも属さない混在群を集約する。

#### `tests/output_player.rs` (4 件、約 320 行、全 `#[cfg(feature = "player")]`)

| 行 | 関数名 |
|---|---|
| 2550 | `start_output_player_with_closed_control_channel_returns_processing_failed` |
| 2592 | `player_lifecycle_stop_updates_output_status` |
| 2678 | `start_output_player_returns_processing_failed_when_subscriber_startup_fails` |
| 2756 | `stale_player_stopped_event_does_not_deactivate_restarted_player` |

サブモジュール全体を `#[cfg(feature = "player")]` で囲む案は採用しない (各関数定義のみに `#[cfg]` を付ける現状方針を維持)。`--no-default-features` 時はファイル自体は存在するが内部関数が全て無効化される。

#### `tests/output_create.rs` (17 件、約 800 行)

| 行 | 関数名 |
|---|---|
| 3056 | `hisui_create_output_stream_reads_stream_service_settings` |
| 3112 | `hisui_create_output_sora_reads_sora_sdk_settings` |
| 3241 | `hisui_create_output_mp4_without_record_directory_uses_default` |
| 3267 | `hisui_create_output_hls_reads_destination_and_variants` |
| 3350 | `hisui_create_output_sora_with_metadata_preserves_it` |
| 3409 | `set_output_settings_rejects_invalid_record_directory_type` |
| 3435 | `set_output_settings_rejects_invalid_sora_metadata_type` |
| 3462 | `set_output_settings_rejects_invalid_signaling_urls_type` |
| 3489 | `set_output_settings_rejects_invalid_stream_service_type` |
| 3514 | `set_output_settings_null_clears_sora_channel_id` |
| 3582 | `set_record_directory_updates_default_for_future_mp4_outputs` |
| 3647 | `hisui_create_output_rejects_invalid_record_directory_type` |
| 3672 | `hisui_create_output_rejects_invalid_stream_service_type` |
| 3697 | `hisui_create_output_rejects_invalid_sora_signaling_urls_type` |
| 3722 | `hisui_create_output_rejects_non_object_output_settings` |
| 3748 | `set_output_settings_rejects_non_object_output_settings` |
| 3789 | `set_output_settings_record_updates_default_record_directory` |

#### `tests/request_batch.rs` (2 件、約 50 行)

| 行 | 関数名 |
|---|---|
| 2863 | `request_batch_with_halt_on_failure_stops_after_first_failure` |
| 2887 | `request_batch_without_halt_on_failure_continues_after_failure` |

#### `tests/persistent_data.rs` (4 件、約 140 行)

| 行 | 関数名 |
|---|---|
| 2915 | `set_persistent_data_rejects_null_slot_value` |
| 2940 | `set_persistent_data_rejects_profile_realm` |
| 2965 | `get_persistent_data_returns_null_for_nonexistent_slot` |
| 2996 | `set_then_get_persistent_data_roundtrip` |

#### `tests/stream_service.rs` (2 件、約 130 行)

| 行 | 関数名 |
|---|---|
| 4609 | `handle_get_stream_service_settings_emits_use_auth_when_key_none` |
| 4674 | `handle_get_stream_service_settings_emits_use_auth_when_key_some` |

#### `tests/text_overlay.rs` (16 件 + 専用ヘルパー 3 件、約 650 行)

| 行 | 関数名 |
|---|---|
| 4065 | `hisui_create_text_overlay_returns_disabled_when_feature_off` |
| 4088 | `hisui_update_text_overlay_returns_disabled_when_feature_off` |
| 4110 | `hisui_remove_text_overlay_returns_disabled_when_feature_off` |
| 4133 | `hisui_list_text_overlays_returns_disabled_when_feature_off` |
| 4160 | `hisui_text_overlay_create_list_update_remove_roundtrip` |
| 4282 | `hisui_create_text_overlay_rejects_duplicate_name` |
| 4315 | `hisui_update_text_overlay_rejects_unknown_name` |
| 4336 | `hisui_remove_text_overlay_rejects_unknown_name` |
| 4357 | `hisui_create_text_overlay_rejects_invalid_font_name` |
| 4390 | `hisui_create_text_overlay_rejects_unresolvable_font` |
| 4413 | `hisui_create_text_overlay_rejects_invalid_color` |
| 4445 | `hisui_create_text_overlay_rejects_invalid_font_size` |
| 4471 | `hisui_create_text_overlay_rejects_invalid_text` |
| 4517 | `hisui_create_text_overlay_returns_missing_request_field_when_required_missing` |
| 4555 | `hisui_create_text_overlay_returns_invalid_request_field_for_type_mismatch` |
| 4592 | `hisui_text_overlay_accepts_i32_max_as_z_value` |

専用ヘルパー 3 件 (`create_initialized_coordinator_with_text_overlay` / `process_text_overlay_request` / `parse_text_overlays_count`) もこのファイルに置く。

#### 合計確認

14 + 7 + 6 + 9 + 6 + 3 + 2 + 7 + 4 + 17 + 2 + 4 + 2 + 16 = **99 件** (実数と一致)。

### 4. 共通ヘルパーの可視性

- `tests/common.rs` 内の関数は `pub(super) fn` または `pub(super) async fn` で公開する。`super = session::tests` (エントリポイント) のため、`session::tests` 配下の全サブモジュール (`scene` / `lifecycle` / etc.) から `super::common::xxx` で参照できる。
- `pub(crate)` は採用しない。crate (`hisui` 本体) 全体に公開する意図はなく、テストヘルパーが他の `#[cfg(test)]` モジュール (例えば `state/tests.rs` や `response/tests.rs`) から `crate::obsws::session::tests::common::default_coordinator_handle()` のように誤参照される懸念があるため。
- `pub(super)` 化対象は **`tests/common.rs` に移動する共通ヘルパー 23 件のみ** (現状すべて private)。`tests/text_overlay.rs` に移動する text_overlay 専用ヘルパー 3 件は同一ファイル内で完結するため private のまま (可視性指定なし) とする。
- `#[cfg(feature = "player")]` 付きヘルパー 3 件は `--no-default-features` 時に関数自体が消える (関数定義に `#[cfg(feature = "player")]` を付けたまま `pub(super)` 化する) ため、dead_code lint は出ない。
- ヘルパー本体・引数・戻り値型・`fn` / `async fn` の別は一切変更しない。可視性の `pub(super)` 化追加のみが許可される変更。

### 5. `use` 宣言とシンボル解決

サブモジュール側で必要な本体シンボルを明示的に `use` する。`use crate::obsws::session::*;` の glob import は意図したシンボル全てを取り込めない (現行 `session.rs:1-9` が `pub use` 再エクスポートをしていない: `CloseCode` / `RequestMessage` / `ClientMessage` / `OBSWS_*` 定数群は `session.rs` 内で `use` されているだけで `pub` は付かないため、外部から `crate::obsws::session::CloseCode` ではアクセスできない)。

各サブモジュールの先頭は以下の形にする (以下は `tests/scene.rs` の **仮例**。実装時に各サブモジュールで実際に使うシンボルを洗い出して必要なものだけを書く):

```rust
use shiguredo_websocket::CloseCode;

use crate::obsws::auth::ObswsAuthentication;
use crate::obsws::message::{ClientMessage, ObswsSessionStats, RequestBatchMessage, RequestMessage};
use crate::obsws::protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_EVENT_SUB_ALL,
};
use crate::obsws::session::{ObswsSession, SessionAction};

use super::common::*;
```

各サブモジュールで実際に必要な `use` は、現行 `tests.rs` 本文 (line 17 以降) で使われているシンボルを機能群ごとに洗い直して必要なものだけを書く (例えば `tests/request_batch.rs` / `tests/stream_service.rs` は `CloseCode` 不要、`tests/text_overlay.rs` は `OBSWS_*` 定数不要)。実装時は各サブモジュールごとに `cargo clippy --workspace --all-targets --all-features -- -D warnings` で `unused_imports` 警告を見ながら整理する。エントリポイント `tests.rs` には `use` 宣言を置かず、`pub use` 再エクスポートもしない (各サブモジュールが必要なシンボルを直接 import する方針)。

### 6. feature gate の取り扱い

- `#[cfg(feature = "player")]` 付きテスト 4 件は `tests/output_player.rs` にまとめる。ファイル内の全関数が player feature 限定であるため、エントリポイント `tests.rs` 側で **モジュール宣言ごと cfg ゲートする**:

  ```rust
  #[cfg(feature = "player")]
  #[path = "tests/output_player.rs"]
  mod output_player;
  ```

  これにより `--no-default-features` 時は `output_player.rs` 自体が読み込まれず、ファイル冒頭の `use super::common::test_player_command_tx;` 等の `--no-default-features` 時に解決失敗するシンボル参照や `unused_imports` lint 衝突を完全に回避できる。`output_player.rs` 内の各関数の `#[cfg(feature = "player")]` 属性は冗長になるが、現状を残しておく (関数単位での見落とし防止としても機能する)。
- `#[cfg(feature = "player")]` 付きヘルパー 3 件 (`create_coordinator_handle_with_player_channels` / `test_player_command_tx` / `test_player_media_tx`) は `tests/common.rs` に置き、関数定義に `#[cfg(feature = "player")]` を付けたまま `pub(super)` 化する。`--no-default-features` 時は関数自体が消えるため dead_code lint は出ない。
- `Cargo.toml` の `default = ["player"]` のため、`--no-default-features` を指定すると `player` が外れる。このとき player テスト 4 件が無効化されるため、`running N tests` の N は default 時より 4 件少なくなる。
- `fdk-aac` 関連の feature gate テストは現状 0 件で本 issue でも追加しない。
- text_overlay は `text_overlay_config: Option<TextOverlayConfig>` で表現されており feature gate には依存しないため `tests/text_overlay.rs` に feature gate は付けない。

### 7. PR / コミット戦略

1 PR で完結させる (純粋なファイル間移動でロジック変更ゼロのため、件数比較と feature 別 pass でリグレッションが無いことを担保すればレビューは差分量に依存せず実施可能)。squash 前提。

コミットは 14 phase × 1 commit ずつに分ける (合計 14 commit)。各 phase は「1 ファイル新規作成 + 該当テスト/ヘルパーの物理移動 + エントリポイント側の該当ブロック削除 + `#[path = ...] mod ...;` 宣言追加 + (必要なら) `use` 文整理」を 1 commit にまとめる。中間 commit で「テスト関数とサブモジュール宣言が混在する」状態を許容するが、各 phase 終了時点で `cargo test --lib obsws::session` が pass し、`running N tests` の N が 99 (default、`--features player` 込み) であることを保証する。

Phase の順序はディレクトリツリー (設計方針 1) と一致させる。

| Phase | 内容 |
|---|---|
| 1 | `tests/common.rs` を新規作成し、共通ヘルパー 23 件を物理移動して `pub(super)` 化する。エントリポイント `tests.rs` に `#[path = "tests/common.rs"] mod common;` を追加し、**作業中暫定** の `use common::*;` を 1 行追加する (まだエントリポイントに残る 99 件のテストがヘルパーを修飾なしで呼び続けられるようにするため)。この暫定 `use common::*;` は Phase 14 で削除する |
| 2 | `tests/lifecycle.rs` を新規作成 + 14 件移動 + `#[path]` 宣言追加 |
| 3 | `tests/scene.rs` を新規作成 + 7 件移動 (line 188 / 236 の混在 2 件込み) + `#[path]` 宣言追加 |
| 4 | `tests/scene_item.rs` を新規作成 + 9 件移動 + `#[path]` 宣言追加 |
| 5 | `tests/input.rs` を新規作成 + 6 件移動 + `#[path]` 宣言追加 |
| 6 | `tests/output_record.rs` を新規作成 + 6 件移動 + `#[path]` 宣言追加 |
| 7 | `tests/output_stream.rs` を新規作成 + 3 件移動 + `#[path]` 宣言追加 |
| 8 | `tests/output_hls_dash.rs` を新規作成 + 2 件移動 + `#[path]` 宣言追加 |
| 9 | `tests/output_misc_lifecycle.rs` を新規作成 + 7 件移動 + `#[path]` 宣言追加 |
| 10 | `tests/output_player.rs` を新規作成 + 4 件移動 + `#[cfg(feature = "player")] #[path = ...] mod output_player;` 宣言追加 (設計方針 6 参照) |
| 11 | `tests/output_create.rs` を新規作成 + 17 件移動 + `#[path]` 宣言追加 |
| 12 | `tests/request_batch.rs` / `tests/persistent_data.rs` / `tests/stream_service.rs` を新規作成 + 計 8 件移動 + `#[path]` 宣言 3 件追加 (3 ファイル合計約 320 行で 1 commit にまとめる。Phase 7/8 が独立 commit なのに対し本 Phase が合体する根拠: いずれも 50-140 行と非常に小さく、機能的に独立しているため `git diff` 視認性も問題ない) |
| 13 | `tests/text_overlay.rs` を新規作成 + 16 件移動 + 専用ヘルパー 3 件移動 + `#[path]` 宣言追加 + **`pbt/tests/prop_text_overlay.rs:10` の doc コメントを `src/obsws/session/tests/text_overlay.rs` に更新** (text_overlay 関連変更を 1 commit に集約) |
| 14 | エントリポイント `tests.rs` の最終整理 (Phase 1 で追加した暫定 `use common::*;`、使われなくなったセパレータコメント、残存空行の削除。最終的に `tests.rs` は `#[path]` + `mod` 宣言の 15 ペアのみで 30 行前後になる) |

### 8. 対象外スコープ

本 issue で扱わない:

- **テスト関数の本体・名前・引数・属性 (`#[tokio::test]` / `#[cfg(feature = "...")]`) の変更**。純粋なファイル間移動のみ。
- **ヘルパー関数の本体・引数・戻り値型の変更**。`tests/common.rs` に移動する 23 件への `pub(super)` 化の可視性追加のみ許可。
- **テスト関数の順序の入れ替え**。各サブモジュール内では、元 `tests.rs` 内での当該機能群テスト関数間の **相対順** (元ファイルで先に出現したテストはサブモジュール内でも先に置く) を保つ。他機能群との絶対位置は分割により失われるため保証しない。
- **`src/obsws/state/tests.rs` (1361 行) と `src/obsws/response/tests.rs` (1389 行) の分割**。両者は本 issue の上限 1500 行内に収まっているため対象外。1500 行を超えた段階で別 issue を起票する。
- **`src/obsws/session/output.rs` (160 行、本体側既存サブモジュール) の変更**。本 issue は `tests/` 配下のみが対象。
- **closed issue 配下の行番号参照 (例: `closed/0042-...md` 内の `src/obsws/session/tests.rs:3093` 等) の更新**。closed issue は historic record として変更しない。
- **テスト名のリネーム / 共通ヘルパー API の整理 / 重複テストの統合**。これらは別 issue で必要に応じて起票する。
- **`output_create.rs` を `record / stream / hls / sora` 等にさらに細分する作業**。本 issue の閾値内 (約 800 行) に収まっているため見送り、1500 行に近づいた段階で別 issue を起票する。

## 完了条件

### 構成検証

- `src/obsws/session/tests.rs` のエントリポイントは `#[path]` 属性付き `mod` 宣言と必要最小限の `#![allow(...)]` (もしあれば) のみで、関数定義・テスト・`use` 宣言を一切含まない (概ね 30 行前後)
- `src/obsws/session/tests/` 配下の 15 ファイル (common + 14 サブモジュール) がすべて 1500 行以下
- `src/obsws/session/tests/common.rs` は 500 行以下
- 「設計方針 3」の振り分け表どおりに 99 件のテスト関数と 26 件のヘルパーが配置されている

### 着手日の再計測

着手日に `src/obsws/session/tests.rs` の行番号や件数がドリフトしている可能性があるため、Phase 1 開始前に以下のコマンドで再計測する:

```bash
wc -l src/obsws/session/tests.rs
grep -c "^#\[tokio::test\]" src/obsws/session/tests.rs
grep -n "^async fn \|^fn " src/obsws/session/tests.rs
grep -n "src/obsws/session/tests.rs" pbt/tests/prop_text_overlay.rs
```

得られた件数と振り分け表合計 (99) が一致するか、追加・削除されたテストがないかを確認する。差分があれば本 issue を再 polish して振り分け表を更新してから Phase 1 へ進む。

### 着手前の最小再現検証 (Phase 1 開始前に行う)

`src/` 配下にネスト `#[path]` の前例が存在しないため、本格実装に入る前に最小再現で動作確認する。`cargo check` でシンボル解決が通ることをもって検証する (`cargo test` での pass は「何も呼んでいない」状態でも成立してしまうため検証としては弱い):

1. **エントリポイント基準の `#[path]` 解決確認**:
   - `tests/common.rs` を以下の 1 関数で作成:
     ```rust
     pub(super) fn dummy() -> i32 { 42 }
     ```
   - エントリポイント `tests.rs` の末尾に以下を追加 (既存の 99 件のテストは触らずに残しておく):
     ```rust
     #[path = "tests/common.rs"]
     mod common;

     #[allow(dead_code)]
     fn _ensure_common_resolution() -> i32 { common::dummy() }
     ```
   - `cargo check --lib --all-features` が pass することを確認する。失敗した場合 `#[path]` 解決が動いていないので、設計方針 2 の別案を検討し本 issue を再 polish する
2. **兄弟サブモジュール間の `super::common::*` 解決確認**:
   - `tests/scene.rs` を以下の 1 関数で作成:
     ```rust
     #[allow(dead_code)]
     pub(super) fn dummy_scene() -> i32 { super::common::dummy() }
     ```
   - エントリポイント `tests.rs` に以下を追加:
     ```rust
     #[path = "tests/scene.rs"]
     mod scene;
     ```
   - `cargo check --lib --all-features` が pass することを確認する。これで「兄弟サブモジュール (`tests::scene`) から `super::common::dummy()` で参照できる = `pub(super)` 可視性で実装方針が成立する」ことが確認できる
3. 両ステップが pass したら、検証用に追加した `_ensure_common_resolution` / `dummy` / `dummy_scene` / `mod common;` / `mod scene;` を一旦削除し、設計方針 7 の Phase 1 (`tests/common.rs` の本実装) へ進む

検証中の小さな commit はインクリメンタルに残してもよいが、squash 前提でまとめる。

### テスト保全検証

リファクタ前後で以下のコマンドの `running N tests` の N が一致すること (N の値は実装着手時に再計測して PR 説明に貼る):

- `cargo test --lib obsws::session` (default = `["player"]` 込み、期待 99 件)
- `cargo test --lib obsws::session --all-features` (期待 99 件)
- `cargo test --lib obsws::session --no-default-features` (期待 95 件、player 4 件減)

リファクタブランチの作業開始 commit と作業完了 commit の両方で同コマンドを実行し、出力差分を PR 説明に貼る。**さらに各 phase の commit 完了時点でも `cargo test --lib obsws::session` の N = 99 (default) が変わらないことを確認** し、変わる commit が出た場合はその場で修正する。

### 静的検査

以下のコマンドが全て pass すること (closed/0042 / closed/0053 と同水準):

- `cargo check --workspace --all-features --tests --benches`
- `cargo check --workspace --no-default-features`
- `cargo test --workspace`
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo clippy --workspace --no-default-features -- -D warnings`

### 関連参照の更新

- `pbt/tests/prop_text_overlay.rs:10` の doc コメントが分割後の新パス (`src/obsws/session/tests/text_overlay.rs`) を指すよう更新されている
- リポジトリ全体で `src/obsws/session/tests.rs` をパスとして参照しているのは 1 箇所のみ (上記) で、他に open な参照は存在しない (本 issue の Polish 時点で `grep -rn "src/obsws/session/tests.rs" --include="*.rs" --include="*.md" --exclude-dir=issues/closed .` で確認済み)

### CHANGES.md

`## develop` に追記しない。テストコード内の構造変更のみで、利用者から見える挙動・公開 API・出力 JSON・state file の永続化フォーマット・テスト関数名は一切変化しないため。

## 関連

- closed issue 0042 (`feature/refactor-unify-stream-service-settings-emitters`): `session/tests.rs:3093` 等への行番号参照があるが、closed issue は historic record として更新しない。本 issue で `handle_get_stream_service_settings_*` 2 テストは `tests/stream_service.rs` へ移動する。
- closed issue 0053 (`feature/refactor-hex-color`): `hisui_create_text_overlay_rejects_invalid_color` 等を参照しているが、closed のため更新しない。該当テストは `tests/text_overlay.rs` へ移動する。
- open issue 0046 / 0052: 「前提条件」節を参照。
