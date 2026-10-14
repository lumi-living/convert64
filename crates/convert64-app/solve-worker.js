// The solve worker (#265; crates/convert64-app/src/scheduler/web.rs): a dedicated module
// worker running the app's own wasm module. Its `main` finds no window and serves solve
// requests from the page instead of starting the user interface. trunk copies this file
// next to convert64-app.js (index.html); Trunk.toml keeps the file names unhashed.
import init from './convert64-app.js';

await init({ module_or_path: new URL('./convert64-app_bg.wasm', import.meta.url) });
