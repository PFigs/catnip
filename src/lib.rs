pub mod app;
pub mod cat;
pub mod github;
pub mod keys;
pub mod model;
pub mod net;
pub mod store;
pub mod view;

use rune::AppSpec;

pub fn run() {
    // The desktop entry `mise run desktop` installs has this name.
    rune::set_app_id("catnip");
    let spec = AppSpec {
        min_size: Some((900.0, 600.0)),
        setup: Some(app::setup),
        poll: Some(app::poll),
        on_key: Some(app::on_key),
        scenes: &[("sample", view::sample), ("booting", view::booting)],
        ..AppSpec::new("Catnip", (1280.0, 800.0), app::init, view::view)
    };
    let mut ui = rune::App::new(spec);
    // A scene shows canned state, which the first request would replace.
    if !scene_run() {
        let handle = ui.handle();
        ui = ui.on_ready(move |_| handle.update(|s: &mut app::App, _| s.start()));
    }
    ui.run();
}

/// The app runs a scene (`--scene <name>`): canned state, no request and no saved settings.
pub(crate) fn scene_run() -> bool {
    std::env::args().any(|a| a == "--scene")
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    run();
}
