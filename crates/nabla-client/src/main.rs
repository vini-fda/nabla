#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

/// Window and Dock icon. macOS shows it exactly as given, so it gets the platform's
/// rounded-square shape and transparent margins; elsewhere the plain square logo is used.
#[cfg(target_os = "macos")]
const APP_ICON: &[u8] = include_bytes!("../assets/app-icon-macos.png");
#[cfg(all(not(target_os = "macos"), not(target_arch = "wasm32")))]
const APP_ICON: &[u8] = include_bytes!("../assets/favicon-512x512.png");

/// Inside a macOS app bundle (`cargo xtask bundle`), the Dock shows the bundle's layered
/// Liquid Glass icon; an empty icon tells eframe not to replace it. Elsewhere use [`APP_ICON`].
#[cfg(not(target_arch = "wasm32"))]
fn app_icon() -> egui::IconData {
    let in_bundle = cfg!(target_os = "macos")
        && std::env::current_exe()
            .is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"));
    if in_bundle {
        egui::IconData::default()
    } else {
        eframe::icon_data::from_png_bytes(APP_ICON).expect("Failed to load icon")
    }
}

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // Log to stderr. Say what with `RUST_LOG`, as in `RUST_LOG=nabla_client=trace`: see `span.rs`.
    // `cargo nabla` asks for the dev format, with timestamps and sources, and more of it.
    if std::env::var("NABLA_LOG_STYLE").is_ok_and(|style| style == "dev") {
        let mut builder = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or(nabla_client::dev_log::DEFAULT_FILTER),
        );
        nabla_client::dev_log::format(&mut builder).init();
    } else {
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("warn,nabla_client=info"),
        )
        .init();
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 850.0])
            .with_min_inner_size([700.0, 600.0])
            .with_maximized(true)
            .with_title("Nabla")
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: nabla_client::DEPTH_BUFFER,
        multisampling: nabla_client::MULTISAMPLING,
        // The bundled fonts (theme.rs) and egui's cover the app's text; eframe is built without
        // its `system_fonts` feature, so there is no system fallback to look for.
        system_font_fallback: false,
        ..Default::default()
    };
    eframe::run_native(
        "Nabla",
        native_options,
        Box::new(|cc| Ok(Box::new(nabla_client::ViewerApp::new(cc)))),
    )
}

/// What to log, as the page's address says, as in `?log=trace`: see `span.rs`.
#[cfg(target_arch = "wasm32")]
fn web_log_level() -> log::LevelFilter {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|query| {
            query
                .trim_start_matches('?')
                .split('&')
                .find_map(|pair| pair.strip_prefix("log=")?.parse().ok())
        })
        .unwrap_or(log::LevelFilter::Info)
}

// When compiling to web using trunk:
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(web_log_level()).ok();

    let web_options = eframe::WebOptions {
        // WebGPU where the browser has it, else WebGL 2 (egui-wgpu's default setup).
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: nabla_client::DEPTH_BUFFER,
        ..Default::default()
    };

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("Failed to find the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id was not a HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(nabla_client::ViewerApp::new(cc)))),
            )
            .await;

        // Remove the loading text and spinner:
        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(
                        "<p> The app has crashed. See the developer console for details. </p>",
                    );
                    panic!("Failed to start eframe: {e:?}");
                }
            }
        }
    });
}
