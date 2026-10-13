//! Milliseconds from a monotonic clock: `std::time::Instant` natively, the browser's
//! `performance.now()` in wasm (where `Instant` panics), through eframe's `web_sys`.

/// Milliseconds since an arbitrary fixed origin.
#[cfg(not(target_arch = "wasm32"))]
pub fn now_ms() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// Milliseconds since the page loaded (0 without a `window`, e.g. in a web worker).
#[cfg(target_arch = "wasm32")]
pub fn now_ms() -> f64 {
    eframe::web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

/// A diagnostic line: standard error natively, the browser console in wasm (through
/// eframe's `web_sys`, whose `console` it enables).
pub fn log(line: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{line}");
    #[cfg(target_arch = "wasm32")]
    eframe::web_sys::console::log_1(&eframe::wasm_bindgen::JsValue::from_str(line));
}
