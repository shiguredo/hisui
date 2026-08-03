//! Hisui DevTools のネイティブ GUI アプリのエントリポイント。

mod ui;

use gpui::{App, Application, WindowOptions, prelude::*};

use crate::ui::DevToolsApp;

fn main() {
    tracing_subscriber::fmt().init();

    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Hisui DevTools".into()),
                    ..Default::default()
                }),
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
                    None,
                    gpui::size(gpui::px(1280.), gpui::px(800.)),
                    cx,
                ))),
                focus: true,
                ..Default::default()
            },
            |_, cx| cx.new(DevToolsApp::new),
        )
        .expect("ウィンドウの作成に失敗しました");

        // 最後のウィンドウが閉じられたらアプリを終了する
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.activate(true);
    });
}
