#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

/// Window and Dock icon. macOS shows it exactly as given, so it gets the platform's
/// rounded-square shape and transparent margins; elsewhere the plain square logo is used.
#[cfg(target_os = "macos")]
const APP_ICON: &[u8] = include_bytes!("../assets/app-icon-macos.png");
#[cfg(all(not(target_os = "macos"), not(target_arch = "wasm32")))]
const APP_ICON: &[u8] = include_bytes!("../assets/favicon-512x512.png");

/// Inside a macOS app bundle (`viewer/macos/bundle.sh`), the Dock shows the bundle's layered
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
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    // Optional argument: a solution source file to open.
    let source = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 850.0])
            .with_min_inner_size([700.0, 450.0])
            .with_title("Nabla")
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        // The app name keys the persisted settings, so it stays; the window title is "Nabla".
        "ship2d viewer",
        native_options,
        Box::new(|cc| Ok(Box::new(ship2d_graphics::ViewerApp::new(cc, source)))),
    )
}

// When compiling to web using trunk:
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions {
        renderer: eframe::Renderer::Glow,
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
                Box::new(|cc| Ok(Box::new(ship2d_graphics::ViewerApp::new(cc, None)))),
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
