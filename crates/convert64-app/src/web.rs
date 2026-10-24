//! The browser page (#241): the tab title, the unsaved-changes guard, and saving files
//! straight to the user's disk. Like the solve worker (#265) it reaches the browser API
//! by name through `js_sys::Reflect` on eframe's re-exports, so the web build needs no
//! further crates or `web-sys` features (TAD §4).

use crate::files::FileEvent;
use eframe::wasm_bindgen::closure::Closure;
use eframe::wasm_bindgen::{JsCast as _, JsValue};
use eframe::web_sys::js_sys::{self, Array, Function, Object, Promise, Reflect, Uint8Array};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    /// The folder the export set last went into (a `FileSystemDirectoryHandle`), for
    /// Export Again while the page is open (UX §6b, #287).
    static EXPORT_DIR: RefCell<Option<JsValue>> = const { RefCell::new(None) };
}

/// `target[name]`.
pub(crate) fn get(target: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    Reflect::get(target, &JsValue::from_str(name))
}

/// Whether `target[name]` is a function.
pub(crate) fn has_method(target: &JsValue, name: &str) -> bool {
    get(target, name).is_ok_and(|f| f.is_function())
}

/// `target[name](...args)`.
pub(crate) fn call(target: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let f = get(target, name)?.dyn_into::<Function>()?;
    Reflect::apply(&f, target, &args.iter().collect::<Array>())
}

/// Awaits `value` as a promise.
pub(crate) async fn settle(value: JsValue) -> Result<JsValue, JsValue> {
    js_sys::futures::JsFuture::from(value.dyn_into::<Promise>()?).await
}

/// `await target[name](...args)`.
pub(crate) async fn call_async(
    target: &JsValue,
    name: &str,
    args: &[JsValue],
) -> Result<JsValue, JsValue> {
    settle(call(target, name, args)?).await
}

/// An object with the given properties.
pub(crate) fn object(props: &[(&str, JsValue)]) -> JsValue {
    let o = Object::new();
    for (k, v) in props {
        let _ = Reflect::set(&o, &JsValue::from_str(k), v);
    }
    o.into()
}

/// A `DOMException`'s (or `Error`'s) name, such as `QuotaExceededError`.
pub(crate) fn error_name(e: &JsValue) -> String {
    get(e, "name")
        .ok()
        .and_then(|n| n.as_string())
        .unwrap_or_default()
}

/// A thrown value as one line: "QuotaExceededError: …".
pub(crate) fn describe(e: &JsValue) -> String {
    let message = get(e, "message").ok().and_then(|m| m.as_string());
    match (error_name(e), message) {
        (n, Some(m)) if !n.is_empty() => format!("{n}: {m}"),
        (_, Some(m)) => m,
        (n, None) if !n.is_empty() => n,
        _ => format!("{e:?}"),
    }
}

/// `bytes` as a `Uint8Array`.
pub(crate) fn bytes(b: &[u8]) -> Uint8Array {
    let a = Uint8Array::new_with_length(b.len() as u32);
    a.copy_from(b);
    a
}

fn window() -> JsValue {
    js_sys::global().into()
}

fn document() -> Result<JsValue, JsValue> {
    get(&window(), "document")
}

/// Sets the tab title (eframe's web backend ignores `ViewportCommand::Title`).
pub fn set_title(title: &str) {
    if let Ok(doc) = document() {
        let _ = Reflect::set(&doc, &"title".into(), &title.into());
    }
}

/// While `dirty` is true, leaving the page (reload, closing the tab, `Cmd W`) asks
/// first: the browser's own "Leave site?" prompt, through `beforeunload`.
pub fn guard_unload(dirty: Rc<Cell<bool>>) {
    let handler = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        if dirty.get() {
            let _ = call(&event, "preventDefault", &[]);
            // Older browsers want a returnValue.
            let _ = Reflect::set(&event, &"returnValue".into(), &"".into());
        }
    });
    let added = call(
        &window(),
        "addEventListener",
        &["beforeunload".into(), handler.as_ref().clone()],
    );
    if let Err(e) = added {
        crate::scheduler::clock::log(&format!(
            "convert64: no unsaved-changes guard: {}",
            describe(&e)
        ));
    }
    // It lives as long as the page.
    handler.forget();
}

/// How one file left the page.
enum Outcome {
    /// Written where the user chose (`showSaveFilePicker`).
    Written,
    /// Handed to the browser as a download (`<a download>`).
    Downloaded,
    /// The user closed the picker.
    Cancelled,
}

/// The picker's `types` for a file named `name`.
fn picker_options(name: &str) -> JsValue {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty() && e.len() <= 15 && e.chars().all(|c| c.is_ascii_alphanumeric()));
    let mut props = vec![("suggestedName", JsValue::from_str(name))];
    if let Some(ext) = ext {
        let accept = object(&[(
            "application/octet-stream",
            Array::of1(&format!(".{ext}").into()).into(),
        )]);
        let kind = object(&[
            ("description", format!(".{ext} file").into()),
            ("accept", accept),
        ]);
        props.push(("types", Array::of1(&kind).into()));
    }
    object(&props)
}

/// Hands `data` to the browser as a download named `name`: a Blob behind an
/// `<a download>` clicked here, in the user's own event turn when that is where the
/// save was asked for (eframe runs a click's frame inside the click).
fn download(name: &str, data: &[u8]) -> Result<(), JsValue> {
    let blob = Reflect::construct(
        &get(&window(), "Blob")?.dyn_into::<Function>()?,
        &Array::of2(
            &Array::of1(&bytes(data)),
            &object(&[("type", "application/octet-stream".into())]),
        ),
    )?;
    let url_api = get(&window(), "URL")?;
    let url = call(&url_api, "createObjectURL", &[blob])?;
    let doc = document()?;
    let a = call(&doc, "createElement", &["a".into()])?;
    Reflect::set(&a, &"href".into(), &url)?;
    Reflect::set(&a, &"download".into(), &name.into())?;
    let style = get(&a, "style")?;
    Reflect::set(&style, &"display".into(), &"none".into())?;
    let body = get(&doc, "body")?;
    call(&body, "appendChild", std::slice::from_ref(&a))?;
    call(&a, "click", &[])?;
    call(&a, "remove", &[])?;
    // Revoked once the browser has had the time to start the download.
    let revoke = Closure::once_into_js(move || {
        let _ = call(&url_api, "revokeObjectURL", &[url]);
    });
    call(&window(), "setTimeout", &[revoke, JsValue::from(60_000)])?;
    Ok(())
}

/// Writes `data` to `handle` (a `FileSystemFileHandle`).
async fn write_handle(handle: &JsValue, data: &[u8]) -> Result<(), JsValue> {
    let writable = call_async(handle, "createWritable", &[]).await?;
    if let Err(e) = call_async(&writable, "write", &[bytes(data).into()]).await {
        let _ = call_async(&writable, "abort", &[]).await;
        return Err(e);
    }
    call_async(&writable, "close", &[]).await.map(|_| ())
}

/// Saves `files` to the user's disk with no overlay (usability review C3): one file
/// with the Save dialog (`showSaveFilePicker`, Chromium), several into a folder the
/// user picks (`showDirectoryPicker`), otherwise, or if the picker is refused, each as
/// a download. The picker is asked for here, synchronously, so it still has the user's
/// key press or click. `done` gets the files that were written or handed over, with
/// their sizes, or nothing if the user cancelled.
pub fn save_files(
    files: Vec<(String, Vec<u8>)>,
    done: impl FnOnce(Result<Option<Vec<(String, usize)>>, String>) + 'static,
) {
    let w = window();
    let picker = match files.as_slice() {
        [(name, _)] if has_method(&w, "showSaveFilePicker") => {
            call(&w, "showSaveFilePicker", &[picker_options(name)]).ok()
        }
        [_, _, ..] if has_method(&w, "showDirectoryPicker") => call(
            &w,
            "showDirectoryPicker",
            &[object(&[("mode", "readwrite".into())])],
        )
        .ok(),
        _ => None,
    };
    js_sys::futures::spawn_local(async move {
        let sizes: Vec<(String, usize)> = files.iter().map(|(n, b)| (n.clone(), b.len())).collect();
        let outcome = save_with(picker, &files).await;
        done(match outcome {
            Ok(Outcome::Cancelled) => Ok(None),
            Ok(Outcome::Written | Outcome::Downloaded) => Ok(Some(sizes)),
            Err(e) => Err(e),
        });
    });
}

async fn save_with(
    picker: Option<JsValue>,
    files: &[(String, Vec<u8>)],
) -> Result<Outcome, String> {
    if let Some(promise) = picker {
        match settle(promise).await {
            Ok(handle) => {
                let written = if let [(_, data)] = files {
                    write_handle(&handle, data).await
                } else {
                    write_into(&handle, files).await
                };
                return written.map(|()| Outcome::Written).map_err(|e| describe(&e));
            }
            Err(e) if error_name(&e) == "AbortError" => return Ok(Outcome::Cancelled),
            // No user activation left, or the picker is not allowed here: download.
            Err(e) => crate::scheduler::clock::log(&format!(
                "convert64: save picker refused ({}); downloading instead",
                describe(&e)
            )),
        }
    }
    for (name, data) in files {
        download(name, data).map_err(|e| describe(&e))?;
    }
    Ok(Outcome::Downloaded)
}

/// Writes each of `files` into `dir`, a `FileSystemDirectoryHandle`.
async fn write_into(dir: &JsValue, files: &[(String, Vec<u8>)]) -> Result<(), JsValue> {
    for (name, data) in files {
        let opts = object(&[("create", true.into())]);
        let file = call_async(dir, "getFileHandle", &[name.into(), opts]).await?;
        write_handle(&file, data).await?;
    }
    Ok(())
}

/// What an export wrote: the folder's name (when it went into one) and each file's
/// name and size, or `None` if the user cancelled.
type Exported = Option<(Option<String>, Vec<(String, usize)>)>;

/// Writes the export set's `files` with no overlay (UX §6b, #287): into a folder the
/// user picks (`showDirectoryPicker`, Chromium), with `again` the folder of the last
/// export while the page is open, otherwise, or if the picker is refused, one download
/// per file. The picker is asked for here, synchronously, so it still has the user's
/// key press or click.
pub fn export_files(
    files: Vec<(String, Vec<u8>)>,
    again: bool,
    done: impl FnOnce(Result<Exported, String>) + 'static,
) {
    let w = window();
    let remembered = if again {
        EXPORT_DIR.with(|d| d.borrow().clone())
    } else {
        None
    };
    let picker = if remembered.is_none() && has_method(&w, "showDirectoryPicker") {
        let opts = object(&[
            ("mode", "readwrite".into()),
            ("id", "convert64-export".into()),
        ]);
        call(&w, "showDirectoryPicker", &[opts]).ok()
    } else {
        None
    };
    js_sys::futures::spawn_local(async move {
        let sizes: Vec<(String, usize)> = files.iter().map(|(n, b)| (n.clone(), b.len())).collect();
        let dir = match (remembered, picker) {
            (Some(dir), _) => Some(dir),
            (None, Some(promise)) => match settle(promise).await {
                Ok(dir) => Some(dir),
                Err(e) if error_name(&e) == "AbortError" => return done(Ok(None)),
                // No user activation left, or the picker is not allowed here: download.
                Err(e) => {
                    crate::scheduler::clock::log(&format!(
                        "convert64: folder picker refused ({}); downloading instead",
                        describe(&e)
                    ));
                    None
                }
            },
            (None, None) => None,
        };
        let Some(dir) = dir else {
            let downloaded = files
                .iter()
                .try_for_each(|(name, data)| download(name, data))
                .map_err(|e| describe(&e));
            return done(downloaded.map(|()| Some((None, sizes))));
        };
        match write_into(&dir, &files).await {
            Ok(()) => {
                let folder = get(&dir, "name").ok().and_then(|n| n.as_string());
                EXPORT_DIR.with(|d| *d.borrow_mut() = Some(dir));
                done(Ok(Some((folder, sizes))));
            }
            Err(e) => {
                // Asked again next time.
                EXPORT_DIR.with(|d| *d.borrow_mut() = None);
                done(Err(describe(&e)));
            }
        }
    });
}

/// [`save_files`] reporting to the app's file inbox as `FileEvent`s.
pub(crate) fn save_to_inbox(
    files: Vec<(String, Vec<u8>)>,
    verb: &'static str,
    tag: crate::files::SaveTag,
    push: impl Fn(FileEvent) + 'static,
    ctx: egui::Context,
) {
    save_files(files, move |result| {
        match result {
            Ok(Some(saved)) => push(FileEvent::Saved(saved, verb, tag)),
            Ok(None) => {}
            Err(e) => push(FileEvent::Failed(format!("Can't save: {e}."))),
        }
        ctx.request_repaint();
    });
}
