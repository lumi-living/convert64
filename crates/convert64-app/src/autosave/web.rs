//! The browser's autosave storage (#241): the origin private file system (OPFS,
//! `navigator.storage.getDirectory()`), one file per entry in a `convert64-autosave`
//! folder; where OPFS is missing or refused (no `createWritable`, a private window),
//! IndexedDB, one object store keyed by entry name. Reached by name through
//! `js_sys::Reflect` (`crate::web`), with no `web-sys` features beyond eframe's.

use super::storage::{self, Batch, Contents, Done, Storage, StorageError};
use crate::web::{bytes, call, call_async, describe, error_name, get, has_method, object, settle};
use eframe::wasm_bindgen::JsValue;
use eframe::wasm_bindgen::closure::Closure;
use eframe::web_sys::js_sys::{self, Array, Promise, Uint8Array};
use std::cell::RefCell;
use std::rc::Rc;

/// The folder (OPFS) and database (IndexedDB) name.
const NAME: &str = "convert64-autosave";
/// IndexedDB's object store.
const STORE: &str = "entries";

/// Which browser storage is in use, chosen when the storage is first read.
#[derive(Clone)]
enum Backend {
    Opfs(JsValue),
    Idb(JsValue),
}

/// The browser's storage for autosaved sessions.
#[derive(Clone, Default)]
pub struct WebStorage {
    backend: Rc<RefCell<Option<Backend>>>,
}

/// The browser's autosave storage, if it has any.
pub fn storage() -> Option<Box<dyn Storage>> {
    let g: JsValue = js_sys::global().into();
    let opfs = get(&g, "navigator")
        .and_then(|n| get(&n, "storage"))
        .is_ok_and(|s| has_method(&s, "getDirectory"));
    let idb = get(&g, "indexedDB").is_ok_and(|i| !i.is_undefined() && !i.is_null());
    (opfs || idb).then(|| Box::new(WebStorage::default()) as Box<dyn Storage>)
}

fn error(e: &JsValue) -> StorageError {
    let message = describe(e);
    if error_name(e) == "QuotaExceededError" {
        StorageError::quota(message)
    } else {
        StorageError::other(message)
    }
}

/// OPFS: the autosave folder, if OPFS can write here.
async fn opfs_dir() -> Result<JsValue, JsValue> {
    let g: JsValue = js_sys::global().into();
    // Safari before 18.4 has OPFS but no `createWritable` on the page.
    let writable = get(&g, "FileSystemFileHandle")
        .and_then(|h| get(&h, "prototype"))
        .is_ok_and(|p| has_method(&p, "createWritable"));
    if !writable {
        return Err(JsValue::from_str("OPFS has no createWritable here"));
    }
    let storage = get(&get(&g, "navigator")?, "storage")?;
    let root = call_async(&storage, "getDirectory", &[]).await?;
    call_async(
        &root,
        "getDirectoryHandle",
        &[NAME.into(), object(&[("create", true.into())])],
    )
    .await
}

async fn opfs_names(dir: &JsValue) -> Result<Vec<String>, JsValue> {
    let it = call(dir, "keys", &[])?;
    let mut names = Vec::new();
    loop {
        let step = call_async(&it, "next", &[]).await?;
        if get(&step, "done")?.as_bool() == Some(true) {
            return Ok(names);
        }
        if let Some(n) = get(&step, "value")?.as_string() {
            names.push(n);
        }
    }
}

async fn opfs_read(dir: &JsValue, name: &str) -> Result<Vec<u8>, JsValue> {
    let handle = call_async(dir, "getFileHandle", &[name.into()]).await?;
    let file = call_async(&handle, "getFile", &[]).await?;
    let buffer = call_async(&file, "arrayBuffer", &[]).await?;
    Ok(Uint8Array::new(&buffer).to_vec())
}

/// Writes one file; OPFS commits a writable stream on `close`, so a failed or
/// interrupted write leaves the old file whole.
async fn opfs_write(dir: &JsValue, name: &str, data: &[u8]) -> Result<(), JsValue> {
    let handle = call_async(
        dir,
        "getFileHandle",
        &[name.into(), object(&[("create", true.into())])],
    )
    .await?;
    let writable = call_async(&handle, "createWritable", &[]).await?;
    if let Err(e) = call_async(&writable, "write", &[bytes(data).into()]).await {
        let _ = call_async(&writable, "abort", &[]).await;
        return Err(e);
    }
    call_async(&writable, "close", &[]).await.map(|_| ())
}

/// An IndexedDB request (or transaction, with `ok` = "complete") as a promise of its
/// `result`.
fn request(req: &JsValue, ok: &str) -> Promise {
    let req = req.clone();
    let ok = format!("on{ok}");
    Promise::new(&mut |resolve, reject| {
        let r = req.clone();
        let on_ok = Closure::once_into_js(move |_: JsValue| {
            let result = get(&r, "result").unwrap_or(JsValue::UNDEFINED);
            let _ = resolve.call1(&JsValue::NULL, &result);
        });
        let r = req.clone();
        let on_err = Closure::once_into_js(move |_: JsValue| {
            let e = get(&r, "error").unwrap_or(JsValue::UNDEFINED);
            let _ = reject.call1(&JsValue::NULL, &e);
        });
        let _ = js_sys::Reflect::set(&req, &ok.as_str().into(), &on_ok);
        let _ = js_sys::Reflect::set(&req, &"onerror".into(), &on_err);
        if ok == "oncomplete" {
            let _ = js_sys::Reflect::set(&req, &"onabort".into(), &on_err);
        }
    })
}

async fn idb_open() -> Result<JsValue, JsValue> {
    let g: JsValue = js_sys::global().into();
    let factory = get(&g, "indexedDB")?;
    let req = call(&factory, "open", &[NAME.into(), JsValue::from(1)])?;
    let r = req.clone();
    let upgrade = Closure::once_into_js(move |_: JsValue| {
        if let Ok(db) = get(&r, "result") {
            let _ = call(&db, "createObjectStore", &[STORE.into()]);
        }
    });
    js_sys::Reflect::set(&req, &"onupgradeneeded".into(), &upgrade)?;
    settle(request(&req, "success").into()).await
}

fn idb_store(db: &JsValue, mode: &str) -> Result<(JsValue, JsValue), JsValue> {
    let tx = call(db, "transaction", &[STORE.into(), mode.into()])?;
    let store = call(&tx, "objectStore", &[STORE.into()])?;
    Ok((tx, store))
}

async fn idb_names(db: &JsValue) -> Result<Vec<String>, JsValue> {
    let (_, store) = idb_store(db, "readonly")?;
    let keys = settle(request(&call(&store, "getAllKeys", &[])?, "success").into()).await?;
    Ok(Array::from(&keys)
        .iter()
        .filter_map(|k| k.as_string())
        .collect())
}

async fn idb_read(db: &JsValue, name: &str) -> Result<Vec<u8>, JsValue> {
    let (_, store) = idb_store(db, "readonly")?;
    let value = settle(request(&call(&store, "get", &[name.into()])?, "success").into()).await?;
    if value.is_undefined() {
        return Err(JsValue::from_str(&format!("{name} is missing")));
    }
    Ok(Uint8Array::new(&value).to_vec())
}

/// The whole batch in one transaction: it is written entirely or not at all.
async fn idb_apply(db: &JsValue, batch: &Batch) -> Result<(), JsValue> {
    let (tx, store) = idb_store(db, "readwrite")?;
    let done = request(&tx, "complete");
    for (name, data) in &batch.writes {
        call(&store, "put", &[bytes(data).into(), name.into()])?;
    }
    for name in &batch.deletes {
        call(&store, "delete", &[name.into()])?;
    }
    settle(done.into()).await.map(|_| ())
}

impl WebStorage {
    fn backend(&self) -> Option<Backend> {
        self.backend.borrow().clone()
    }

    /// OPFS if it works here, else IndexedDB.
    async fn choose(&self) -> Result<Backend, JsValue> {
        if let Some(b) = self.backend() {
            return Ok(b);
        }
        let chosen = match opfs_dir().await {
            Ok(dir) => Backend::Opfs(dir),
            Err(e) => {
                crate::scheduler::clock::log(&format!(
                    "convert64: autosave: no OPFS ({}); using IndexedDB",
                    describe(&e)
                ));
                Backend::Idb(idb_open().await?)
            }
        };
        crate::scheduler::clock::log(&format!(
            "convert64: autosave in {}",
            match chosen {
                Backend::Opfs(_) => "OPFS",
                Backend::Idb(_) => "IndexedDB",
            }
        ));
        *self.backend.borrow_mut() = Some(chosen.clone());
        Ok(chosen)
    }
}

async fn read_all(backend: &Backend, names: &[String]) -> Result<Vec<Vec<u8>>, JsValue> {
    let mut out = Vec::with_capacity(names.len());
    for n in names {
        out.push(match backend {
            Backend::Opfs(dir) => opfs_read(dir, n).await?,
            Backend::Idb(db) => idb_read(db, n).await?,
        });
    }
    Ok(out)
}

impl Storage for WebStorage {
    fn load(&self, done: Done<Contents>) {
        let this = self.clone();
        js_sys::futures::spawn_local(async move {
            let result = async {
                let backend = this.choose().await?;
                let names = match &backend {
                    Backend::Opfs(dir) => opfs_names(dir).await?,
                    Backend::Idb(db) => idb_names(db).await?,
                };
                let (records, blobs) = storage::classify(names);
                let records = read_all(&backend, &records).await?;
                Ok::<_, JsValue>(Contents { records, blobs })
            }
            .await;
            done(result.map_err(|e| error(&e)));
        });
    }

    fn read(&self, names: Vec<String>, done: Done<Vec<Vec<u8>>>) {
        let this = self.clone();
        js_sys::futures::spawn_local(async move {
            let result = match this.backend() {
                Some(b) => read_all(&b, &names).await.map_err(|e| error(&e)),
                None => Err(StorageError::other("autosave storage not open")),
            };
            done(result);
        });
    }

    fn apply(&self, batch: Batch, done: Done<()>) {
        let this = self.clone();
        js_sys::futures::spawn_local(async move {
            let result = match this.backend() {
                Some(Backend::Opfs(dir)) => {
                    let mut r = Ok(());
                    for (name, data) in &batch.writes {
                        if let Err(e) = opfs_write(&dir, name, data).await {
                            r = Err(error(&e));
                            break;
                        }
                    }
                    if r.is_ok() {
                        for name in &batch.deletes {
                            // A missing entry (NotFoundError) is fine.
                            let _ = call_async(&dir, "removeEntry", &[name.into()]).await;
                        }
                    }
                    r
                }
                Some(Backend::Idb(db)) => idb_apply(&db, &batch).await.map_err(|e| error(&e)),
                None => Err(StorageError::other("autosave storage not open")),
            };
            done(result);
        });
    }

    fn describe(&self) -> String {
        match self.backend() {
            Some(Backend::Opfs(_)) => "OPFS".to_owned(),
            Some(Backend::Idb(_)) => "IndexedDB".to_owned(),
            None => "browser storage".to_owned(),
        }
    }
}
