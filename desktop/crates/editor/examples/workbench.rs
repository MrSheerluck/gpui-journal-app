use dainiki_editor::EditorTheme;
use gpui::{
    App, Bounds, Context, Render, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, size,
};
use gpui_platform::application;

struct Workbench {
    theme: EditorTheme,
}

impl Render for Workbench {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(self.theme.background)
    }
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(900.0), px(640.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Dainiki - Workbench".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(480.0), px(320.0))),
                ..Default::default()
            },
            |_window, cx| {
                cx.new(|_cx| Workbench {
                    theme: EditorTheme::default(),
                })
            },
        )
        .expect("failed to open workbench window");
        cx.activate(true);
    });
}
