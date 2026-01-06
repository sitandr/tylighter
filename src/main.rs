use std::{iter, str::FromStr, sync::{LazyLock, Mutex, OnceLock, atomic::{AtomicUsize, Ordering}}};

use js_sys::Uint8Array;
use typst::{Document, Feature, Library, LibraryExt, World, WorldExt, diag::{FileResult, Severity, SourceDiagnostic}, foundations::{Bytes, Content, Datetime, Label, Selector, Value}, syntax::{FileId, Source, VirtualPath, package::PackageSpec}, text::{Font, FontBook}, utils::{LazyHash, PicoStr}};
use typst_html::{HtmlDocument, html};
use typst_kit::fonts::{FontSearcher, FontSlot};
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};
use std::collections::HashMap;

static WORLD: LazyLock<WebWorld> = LazyLock::new(|| {
    WebWorld::new()
});

static CACHE_SIZE: AtomicUsize = AtomicUsize::new(10);

static REQUEST_SOURCE: OnceLock<TrashUnsafeWrapper> = OnceLock::new();
const METADATA_LABEL: PicoStr = PicoStr::constant("interact-var");

#[derive(Debug)]
struct TrashUnsafeWrapper {
    f: js_sys::Function
}

/// Please-please-please just don't use multiple threads there...
unsafe impl Sync for TrashUnsafeWrapper {}
unsafe impl Send for TrashUnsafeWrapper {}

struct WebWorld {
    library_hash: LazyHash<Library>,
    fonts: Vec<FontSlot>,
    book: LazyHash<FontBook>,
    files: Mutex<HashMap<FileId, FileSlot>>,
}

impl WebWorld {
    pub fn new() -> Self {
        let library = Library::builder().with_features(iter::once(Feature::Html).collect()).build();
        let mut fonts = FontSearcher::new();
        fonts.include_system_fonts(false);
        let fonts = fonts.search();
        let book = LazyHash::new(fonts.book);
        WebWorld{library_hash: LazyHash::new(library), fonts: fonts.fonts, book, files: Mutex::new(HashMap::new())}
    }
}

impl WebWorld {
    /// Access the canonical slot for the given file id.
    fn slot<F, T>(&self, id: FileId, f: F) -> T
    where
        F: FnOnce(&mut FileSlot) -> T,
    {
        let mut map = self.files.lock().unwrap();
        f(map.entry(id).or_insert_with(|| FileSlot::new(id)))
    }
}

struct FileSlot {
    id: FileId,
    bytes: Option<Bytes>,
    source: Option<Source>
}

fn get_file(id: FileId) -> Vec<u8> {
    let value = REQUEST_SOURCE.get().expect("Retrive function not set").f.call2(&JsValue::null(),
        &JsValue::from_str(&id.package().map(|s| s.to_string()).unwrap_or_default()),
        &JsValue::from_str(&id.vpath().as_rootless_path().to_str().unwrap())).expect("JS returned something?");
    Uint8Array::to_vec(&Uint8Array::new(&value))
}

impl FileSlot {
    fn new(id: FileId) -> Self {
        Self { bytes: None, source: None, id }
    }

    fn set_bytes(&mut self, bytes: Vec<u8>) -> FileResult<&Bytes> {
        self.bytes = Some(Bytes::new(bytes));
        Ok(self.bytes.as_ref().unwrap())
    }

    fn set_source(&mut self, bytes: &[u8]) -> FileResult<Source> {
        self.source = Some(Source::new(self.id, decode_utf8(bytes)?.to_string()));
        Ok(self.source.clone().unwrap())
    }

    fn get_source(&mut self) -> FileResult<Source> {
        if self.source.is_some() {
            Ok(self.source.clone().unwrap())
        } else {
            let data = &get_file(self.id);
            self.set_source(data)
        }
    }

    fn get_bytes(&mut self) -> FileResult<Bytes> {
        if self.bytes.is_some() {
            Ok(self.bytes.clone().unwrap())
        } else {
            let data = get_file(self.id);
            self.set_bytes(data).cloned()
        }
    }
}

/// Decode UTF-8 with an optional BOM.
fn decode_utf8(buf: &[u8]) -> FileResult<&str> {
    // Remove UTF-8 BOM.
    Ok(std::str::from_utf8(buf.strip_prefix(b"\xef\xbb\xbf").unwrap_or(buf))?)
}

impl World for WebWorld {
    fn library(&self) ->  &LazyHash<Library>  {
        &self.library_hash
    }

    #[doc = " Metadata about all known fonts."]
    fn book(&self) ->  &LazyHash<FontBook>  {
        &self.book
    }

    fn main(&self) -> FileId {
        FileId::new(None, VirtualPath::new("main.typ"))
    }

    fn source(&self, id:FileId) -> FileResult<Source>  {
        self.slot(id, |f| f.get_source())
    }

    fn file(&self, id:FileId) -> FileResult<Bytes>  {
        self.slot(id, |f| f.get_bytes())
    }

    fn font(&self,index:usize) -> Option<Font>  {
        self.fonts.get(index)?.get()
    }

    fn today(&self, _offset:Option<i64>) -> Option<Datetime>  {
        Datetime::from_ymd(2025, 1, 1)
    }
}

fn recompile_and_get_metadata() -> Result<(String, Option<String>), typst::ecow::EcoVec<typst::diag::SourceDiagnostic>> {
    let result = typst::compile(&*WORLD);
    let result: Result<(String, Option<String>), _> = result.output.and_then(|r: HtmlDocument| {
        let html = html(&r);
        html.map(|h| {
            let interactive_metadata = r.introspector().query_first(&Selector::Label(Label::new(METADATA_LABEL).unwrap()));
            let interactive_metadata = interactive_metadata.and_then(|m: Content| m.field_by_name("value").ok()).and_then(|m: Value| m.cast().ok());
            (h, interactive_metadata)
        })
    });
    comemo::evict(CACHE_SIZE.load(Ordering::Relaxed));

    result
}

#[wasm_bindgen(getter_with_clone)]
pub struct CompileResult {
    pub html: String,
    pub metadata: Option<String>
}

impl CompileResult {
    fn new(result: (String, Option<String>)) -> Self {
        CompileResult { html: result.0, metadata: result.1 }
    }
}

#[wasm_bindgen(getter_with_clone)]
pub struct ErrorSpan {
    pub severity: bool,
    /// The span of the relevant node in the source code.
    /// pub span: (FileId: (package + path), range),
    /// A diagnostic message describing the problem.
    pub message: String,
    /// The trace of function calls leading to the problem.
    //  pub trace: Vec<Spanned<Tracepoint>>,
    /// Additional hints to the user, indicating how this problem could be avoided
    /// or worked around.
    pub hints: Vec<String>
}

impl ErrorSpan {
    fn from_diagnostic(s: &SourceDiagnostic) -> Self {
        Self {
            severity: s.severity == Severity::Error,
            message: format!("{} {} {:?} {}: ", s.span.id().and_then(|id| id.package()).map(|p| p.to_string()).unwrap_or_default(), s.span.id().map(|id| id.vpath()).unwrap_or(&VirtualPath::new("")).as_rootless_path().display(), WORLD.range(s.span), s.message),
            hints: s.hints.iter().map(ToString::to_string).collect(),
        }
    }
}

#[wasm_bindgen]
pub fn update_file(package_name: String, file_name: String, data: Box<[u8]>) -> Result<(), String> {
    let ps = if !package_name.is_empty() {Some(PackageSpec::from_str(&package_name).map_err(|es| es.to_string())?)} else {None};
    let id = FileId::new(ps, VirtualPath::new(&file_name));

    if !WORLD.files.lock().unwrap().contains_key(&id) {
        return Ok(())//Err(format!("The file {} {} is not requested", package_name, file_name));
    };

    WORLD.slot(id, |file| {
        if file.source.is_some() {
            file.set_source(&data).and(Ok(())).map_err(|err| err.to_string())?
        }
        if file.bytes.is_some() {
            file.set_bytes(data.into_iter().collect()).and(Ok(())).map_err(|err| err.to_string())?
        }
        Result::<(), String>::Ok(())
    })?;
    Ok(())
}

#[wasm_bindgen]
pub fn js_recompile() -> Result<CompileResult, Vec<ErrorSpan>> {
    recompile_and_get_metadata().map_err(|e| e.iter().map(ErrorSpan::from_diagnostic).collect()).map(CompileResult::new)
}

#[wasm_bindgen]
pub fn set_request_f(f: js_sys::Function) {
    REQUEST_SOURCE.set(TrashUnsafeWrapper { f }).unwrap()
}

#[wasm_bindgen]
pub fn set_cache_size(cache_size: u32) {
    CACHE_SIZE.store(cache_size as usize, Ordering::Relaxed);
}

/* 
#[wasm_bindgen]
pub fn set_cache_size(size: u32) {
    // CACHE_SIZE.store(size as usize, Ordering::Relaxed);
}
*/


fn main() {

}
