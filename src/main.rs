//! 像素星舰英雄合成计算器 — Rust GUI 版入口。

mod app;
mod model;
mod search;

use app::{App, MainWindowHandle};
use gpui_kit::component::TitleBar;
use gpui_kit::{
    application, assets, init, open_window, px, size, AppContext as _, WindowBounds, WindowOptions,
};

fn main() {
    application().with_assets(assets::AllAssets).run(|cx| {
        init(cx);
        // 默认日间模式。
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Light, None, cx);

        let bounds = gpui_kit::Bounds::centered(None, size(px(1200.), px(800.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..TitleBar::window_options()
        };
        let (handle, _) = open_window(options, cx, |window, cx| cx.new(|cx| App::new(window, cx)))
            .expect("无法打开主窗口");
        cx.set_global(MainWindowHandle(handle));
    });
}
