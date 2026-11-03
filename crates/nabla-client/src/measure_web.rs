//! The platform-dependent measurements of `nabla_sim::measure`, for the page's script to call,
//! so that the browser's numbers can be set against the native ones
//! (`window.wasmBindings.measure_report(1, 7200, 3000)`).

use wasm_bindgen::prelude::wasm_bindgen;

/// The report of `nabla_sim::measure::report` in this browser.
#[wasm_bindgen]
pub fn measure_report(seed: u32, ticks: u32, steps: u32) -> String {
    nabla_sim::measure::report(u64::from(seed), ticks as usize, steps as usize)
}
