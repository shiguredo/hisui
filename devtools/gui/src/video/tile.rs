//! 映像トラック 1 本分の GPUI タイル。
//!
//! [`RenderImage`] を `img` で描画する。ドラッグで移動でき、右下のハンドルでリサイズできる。

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{App, Entity, ImageSource, IntoElement, ObjectFit, RenderImage, Window, div, img, rgb};

use super::VideoDisplay;

/// 映像の移動ドラッグの状態。
///
/// `on_drag` で設定し、`on_drag_move` で参照する。
/// リサイズと別の型にすることで、ドラッグ種別を型で判別する。
#[derive(Clone)]
struct VideoMoveState {
    track_id: String,
}

/// 映像のリサイズドラッグの状態。
///
/// `on_drag` で設定し、`on_drag_move` で参照する。
/// 移動と別の型にすることで、ドラッグ種別を型で判別する。
#[derive(Clone)]
struct VideoResizeState {
    track_id: String,
}

/// 映像トラック 1 枚を描画する GPUI コンポーネント。
#[derive(IntoElement)]
pub(super) struct VideoTile {
    display: Entity<VideoDisplay>,
    track_id: String,
    image: Arc<RenderImage>,
    position: gpui::Point<f32>,
    size: gpui::Size<f32>,
}

impl VideoTile {
    /// 表示中のフレームからタイルを作る。
    pub(super) fn new(
        display: Entity<VideoDisplay>,
        track_id: String,
        image: Arc<RenderImage>,
        position: gpui::Point<f32>,
        size: gpui::Size<f32>,
    ) -> Self {
        Self {
            display,
            track_id,
            image,
            position,
            size,
        }
    }
}

impl RenderOnce for VideoTile {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let display_for_move = self.display.clone();
        let display_for_drag = display_for_move.clone();
        let track_id = self.track_id.clone();
        div()
            .absolute()
            .left(gpui::px(self.position.x))
            .top(gpui::px(self.position.y))
            .w(gpui::px(self.size.width))
            .h(gpui::px(self.size.height))
            .bg(rgb(0x111111))
            .overflow_hidden()
            .id(gpui::ElementId::Name(gpui::SharedString::from(format!(
                "video-tile-{track_id}"
            ))))
            .on_drag(
                VideoMoveState {
                    track_id: track_id.clone(),
                },
                move |_, _offset, window, cx| {
                    // offset は要素 origin からの相対位置のため、
                    // ドラッグ開始位置はウィンドウ座標のマウス位置を記録する
                    display_for_drag.update(cx, |display, _cx| {
                        display.begin_drag(window.mouse_position().map(f32::from));
                    });
                    cx.new(|_| gpui::EmptyView)
                },
            )
            .on_drag_move({
                let display = display_for_move.clone();
                move |event, _window, cx| {
                    let drag: &VideoMoveState = event.drag(cx);
                    let track_id = drag.track_id.clone();
                    let position = event.event.position.map(f32::from);
                    display.update(cx, |display, cx| {
                        display.move_tile(&track_id, position);
                        cx.notify();
                    });
                }
            })
            .child(
                img(ImageSource::Render(self.image))
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .id(gpui::ElementId::Name(gpui::SharedString::from(format!(
                        "video-{track_id}"
                    )))),
            )
            .child(resize_handle(self.display, track_id))
    }
}

/// 映像タイルの右下に表示するリサイズハンドル。ドラッグでサイズを変更できる。
fn resize_handle(display: Entity<VideoDisplay>, track_id: String) -> impl IntoElement {
    let display_for_resize = display.clone();
    let display_for_drag = display_for_resize.clone();
    div()
        .absolute()
        .right_0()
        .bottom_0()
        .w(gpui::px(16.))
        .h(gpui::px(16.))
        .bg(rgb(0x2b5a2b))
        .id(gpui::ElementId::Name(gpui::SharedString::from(format!(
            "resize-handle-{track_id}"
        ))))
        .on_drag(
            VideoResizeState {
                track_id: track_id.clone(),
            },
            move |_, _offset, window, cx| {
                // offset は要素 origin からの相対位置のため、
                // ドラッグ開始位置はウィンドウ座標のマウス位置を記録する
                display_for_drag.update(cx, |display, _cx| {
                    display.begin_drag(window.mouse_position().map(f32::from));
                });
                cx.new(|_| gpui::EmptyView)
            },
        )
        .on_drag_move({
            move |event, _window, cx| {
                let drag: &VideoResizeState = event.drag(cx);
                let track_id = drag.track_id.clone();
                let position = event.event.position.map(f32::from);
                display_for_resize.update(cx, |display, cx| {
                    display.resize_tile(&track_id, position);
                    cx.notify();
                });
            }
        })
}
