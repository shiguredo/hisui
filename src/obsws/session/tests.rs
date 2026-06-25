// Phase 1 で `tests/common.rs` に共通ヘルパー 23 件を物理移動した。
// 暫定の `use common::*;` はエントリポイント直下に残るテストが
// ヘルパーを修飾なしで呼び続けられるようにするもの。
// 全テストが各サブモジュールへ移動完了する Phase 14 で `mod common;` 含めて整理する。
#[path = "tests/common.rs"]
mod common;
#[path = "tests/input.rs"]
mod input;
#[path = "tests/lifecycle.rs"]
mod lifecycle;
#[path = "tests/output_create.rs"]
mod output_create;
#[path = "tests/output_hls_dash.rs"]
mod output_hls_dash;
#[path = "tests/output_misc_lifecycle.rs"]
mod output_misc_lifecycle;
#[cfg(feature = "player")]
#[path = "tests/output_player.rs"]
mod output_player;
#[path = "tests/output_record.rs"]
mod output_record;
#[path = "tests/output_stream.rs"]
mod output_stream;
#[path = "tests/persistent_data.rs"]
mod persistent_data;
#[path = "tests/request_batch.rs"]
mod request_batch;
#[path = "tests/scene.rs"]
mod scene;
#[path = "tests/scene_item.rs"]
mod scene_item;
#[path = "tests/stream_service.rs"]
mod stream_service;
#[path = "tests/text_overlay.rs"]
mod text_overlay;
