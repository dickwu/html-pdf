//! Append the pages of other PDFs to a PDF (the "attach the uploaded PDFs to
//! the rendered export" use case).
//!
//! Every page is copied with the objects it reaches (content streams, fonts,
//! images, annotations), renumbered into the target document. Attributes a
//! page inherits from its source page tree (`Resources`, `MediaBox`,
//! `CropBox`, `Rotate`) are written onto the copied page itself, so it looks
//! the same under the target's page tree. Document-level structures of the
//! appended PDF (outlines, form field tree, named destinations, structure
//! tree, optional-content configuration) are not merged.
//!
//! An input is fully parsed and staged before the target is touched, so a
//! rejected input leaves the merger exactly as it was.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;

use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId};

/// Inputs above this size are rejected unless the limit is raised.
pub const DEFAULT_MAX_INPUT_BYTES: usize = 128 * 1024 * 1024;
/// The merged document may not grow beyond this many pages unless raised.
pub const DEFAULT_MAX_PAGES: usize = 5_000;
/// Per-stream bound for object/xref streams decoded while loading (bomb guard).
const MAX_DECOMPRESSED_STREAM: usize = 256 * 1024 * 1024;
/// How far into the input the `%PDF-` header may start.
const HEADER_SEARCH_WINDOW: usize = 1024;
/// Guards the walk up a page's `/Parent` chain against cycles.
const MAX_TREE_DEPTH: usize = 64;
const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
/// US Letter, used when neither a page nor its ancestors declare a `MediaBox`.
const FALLBACK_MEDIA_BOX: [i64; 4] = [0, 0, 612, 792];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidKind {
    Empty,
    NotPdf,
    Corrupt,
    NoPages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptedKind {
    /// Needs a user (open) password.
    Password,
    /// Opens without a password but carries owner restrictions.
    OwnerRestricted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    InputBytes,
    Pages,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    Invalid(InvalidKind, String),
    Encrypted(EncryptedKind, String),
    Limit(LimitKind, String),
}

impl MergeError {
    pub fn message(&self) -> &str {
        match self {
            Self::Invalid(_, m) | Self::Encrypted(_, m) | Self::Limit(_, m) => m,
        }
    }
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_pages: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_pages: DEFAULT_MAX_PAGES,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Merger {
    doc: Document,
    pages_root: ObjectId,
    page_count: usize,
    limits: Limits,
    allow_owner_restricted: bool,
}

/// A parsed, validated input ready to be copied in.
struct Source {
    doc: Document,
    pages: Vec<ObjectId>,
}

impl Merger {
    /// Start from `base`; its pages come first in the output.
    pub fn new(base: &[u8]) -> Result<Self, MergeError> {
        let limits = Limits::default();
        let source = load(base, "base PDF", &limits, false)?;
        let mut doc = source.doc;
        let page_count = source.pages.len();
        if page_count > limits.max_pages {
            return Err(too_many_pages(page_count, limits.max_pages));
        }
        let pages_root = pages_root(&doc).map_err(|m| corrupt("base PDF", &m))?;
        doc.max_id = doc.max_id.max(max_object_number(&doc));
        Ok(Self {
            doc,
            pages_root,
            page_count,
            limits,
            allow_owner_restricted: false,
        })
    }

    pub fn page_count(&self) -> usize {
        self.page_count
    }

    pub fn set_max_input_bytes(&mut self, bytes: usize) {
        self.limits.max_input_bytes = bytes;
    }

    pub fn set_max_pages(&mut self, pages: usize) {
        self.limits.max_pages = pages;
    }

    /// Accept PDFs that open without a password but carry an owner password
    /// (permission restrictions). Off by default: copying their pages drops
    /// the restrictions.
    pub fn set_allow_owner_restricted(&mut self, enabled: bool) {
        self.allow_owner_restricted = enabled;
    }

    /// Append every page of `pdf`; returns the number of pages added. On
    /// error nothing changes.
    pub fn append(&mut self, pdf: &[u8]) -> Result<usize, MergeError> {
        let source = load(pdf, "input", &self.limits, self.allow_owner_restricted)?;
        let added = source.pages.len();
        let total = self.page_count + added;
        if total > self.limits.max_pages {
            return Err(too_many_pages(total, self.limits.max_pages));
        }
        let staged = stage(&source, self.doc.max_id + 1, self.pages_root)
            .map_err(|m| corrupt("input", &m))?;
        // Resolve the target's Kids array before any object goes in, so a
        // failure here also leaves the merger untouched.
        let kids_holder =
            kids_holder(&self.doc, self.pages_root).map_err(|m| corrupt("base PDF", &m))?;

        for (id, object) in staged.objects {
            self.doc.objects.insert(id, object);
        }
        self.doc.max_id = staged.next_id - 1;
        let kids = match kids_holder {
            KidsHolder::Inline => self
                .doc
                .get_dictionary_mut(self.pages_root)
                .ok()
                .and_then(|d| d.get_mut(b"Kids").ok()),
            KidsHolder::Indirect(id) => self.doc.get_object_mut(id).ok(),
        };
        if let Some(Object::Array(kids)) = kids {
            kids.extend(staged.pages.iter().map(|&id| Object::Reference(id)));
        }
        if let Ok(root) = self.doc.get_dictionary_mut(self.pages_root) {
            root.set(
                "Count",
                Object::Integer(i64::try_from(total).unwrap_or(i64::MAX)),
            );
        }
        if source.doc.version > self.doc.version {
            self.doc.version.clone_from(&source.doc.version);
        }
        self.page_count = total;
        Ok(added)
    }

    /// The merged document. The merger is left as it was, so more PDFs can
    /// still be appended.
    pub fn to_pdf(&self) -> Result<Vec<u8>, String> {
        let mut doc = self.doc.clone();
        // Object and xref streams of the inputs are rewritten by the writer;
        // the loaded containers would only be dead weight.
        let containers: Vec<ObjectId> = doc
            .objects
            .iter()
            .filter(|(_, object)| matches!(object.type_name(), Ok(b"ObjStm" | b"XRef")))
            .map(|(&id, _)| id)
            .collect();
        for id in containers {
            doc.objects.remove(&id);
        }
        doc.prune_objects();
        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|err| format!("cannot write merged PDF: {err}"))?;
        Ok(out)
    }
}

/// Merge `pdfs` in order into one document.
pub fn merge_all(pdfs: &[&[u8]]) -> Result<Vec<u8>, MergeError> {
    let Some((first, rest)) = pdfs.split_first() else {
        return Err(MergeError::Invalid(
            InvalidKind::Empty,
            "no PDFs to merge: pass at least one".to_string(),
        ));
    };
    let mut merger = Merger::new(first).map_err(|e| at_index(e, 0))?;
    for (index, pdf) in rest.iter().enumerate() {
        merger.append(pdf).map_err(|e| at_index(e, index + 1))?;
    }
    merger
        .to_pdf()
        .map_err(|m| MergeError::Invalid(InvalidKind::Corrupt, m))
}

fn at_index(err: MergeError, index: usize) -> MergeError {
    let prefix = |m: String| format!("PDF #{index}: {m}");
    match err {
        MergeError::Invalid(k, m) => MergeError::Invalid(k, prefix(m)),
        MergeError::Encrypted(k, m) => MergeError::Encrypted(k, prefix(m)),
        MergeError::Limit(k, m) => MergeError::Limit(k, prefix(m)),
    }
}

fn corrupt(what: &str, detail: &str) -> MergeError {
    MergeError::Invalid(InvalidKind::Corrupt, format!("{what} is corrupt: {detail}"))
}

fn too_many_pages(total: usize, max: usize) -> MergeError {
    MergeError::Limit(
        LimitKind::Pages,
        format!("merged PDF would have {total} pages, more than the limit of {max}"),
    )
}

fn load(
    bytes: &[u8],
    what: &str,
    limits: &Limits,
    allow_owner: bool,
) -> Result<Source, MergeError> {
    if bytes.is_empty() {
        return Err(MergeError::Invalid(
            InvalidKind::Empty,
            format!("{what} is empty (0 bytes)"),
        ));
    }
    if bytes.len() > limits.max_input_bytes {
        return Err(MergeError::Limit(
            LimitKind::InputBytes,
            format!(
                "{what} is {} bytes, more than the limit of {} bytes",
                bytes.len(),
                limits.max_input_bytes
            ),
        ));
    }
    let window = &bytes[..bytes.len().min(HEADER_SEARCH_WINDOW)];
    if !window.windows(5).any(|w| w == b"%PDF-") {
        return Err(MergeError::Invalid(
            InvalidKind::NotPdf,
            format!("{what} is not a PDF (no %PDF- header)"),
        ));
    }
    let options = LoadOptions {
        max_decompressed_size: Some(MAX_DECOMPRESSED_STREAM),
        ..LoadOptions::default()
    };
    // lopdf can panic on some malformed input; never let that reach PHP.
    let loaded = std::panic::catch_unwind(|| Document::load_mem_with_options(bytes, options));
    let doc = match loaded {
        Ok(Ok(doc)) => doc,
        Ok(Err(err)) => return Err(corrupt(what, &err.to_string())),
        Err(_) => return Err(corrupt(what, "the parser could not read it")),
    };
    if doc.is_encrypted() {
        return Err(MergeError::Encrypted(
            EncryptedKind::Password,
            format!("{what} is password-protected: it needs a password to open"),
        ));
    }
    if doc.was_encrypted() && !allow_owner {
        return Err(MergeError::Encrypted(
            EncryptedKind::OwnerRestricted,
            format!("{what} is encrypted with owner restrictions (no open password)"),
        ));
    }
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    if pages.is_empty() {
        return Err(MergeError::Invalid(
            InvalidKind::NoPages,
            format!("{what} has no pages"),
        ));
    }
    for (index, &page) in pages.iter().enumerate() {
        check_page(&doc, page).map_err(|m| corrupt(what, &format!("page {}: {m}", index + 1)))?;
    }
    Ok(Source { doc, pages })
}

/// A page must be a dictionary whose content streams all exist.
fn check_page(doc: &Document, page: ObjectId) -> Result<(), String> {
    let dict = doc
        .get_dictionary(page)
        .map_err(|_| "page object is missing or not a dictionary".to_string())?;
    let Ok(contents) = dict.get(b"Contents") else {
        return Ok(()); // A blank page.
    };
    let contents = match contents {
        Object::Reference(id) => doc
            .get_object(*id)
            .map_err(|_| format!("content stream {} {} R is missing", id.0, id.1))?,
        other => other,
    };
    let streams: Vec<&Object> = match contents {
        Object::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    for item in streams {
        let resolved = match item {
            Object::Reference(id) => doc
                .get_object(*id)
                .map_err(|_| format!("content stream {} {} R is missing", id.0, id.1))?,
            other => other,
        };
        if resolved.as_stream().is_err() {
            return Err("page contents are not a stream".to_string());
        }
    }
    Ok(())
}

fn pages_root(doc: &Document) -> Result<ObjectId, String> {
    doc.catalog()
        .map_err(|e| format!("no document catalog ({e})"))?
        .get(b"Pages")
        .and_then(Object::as_reference)
        .map_err(|_| "catalog has no /Pages reference".to_string())
}

enum KidsHolder {
    Inline,
    Indirect(ObjectId),
}

fn kids_holder(doc: &Document, root: ObjectId) -> Result<KidsHolder, String> {
    let dict = doc
        .get_dictionary(root)
        .map_err(|e| format!("page tree root: {e}"))?;
    match dict.get(b"Kids") {
        Ok(Object::Array(_)) => Ok(KidsHolder::Inline),
        Ok(Object::Reference(id)) if matches!(doc.get_object(*id), Ok(Object::Array(_))) => {
            Ok(KidsHolder::Indirect(*id))
        }
        _ => Err("page tree root has no /Kids array".to_string()),
    }
}

fn max_object_number(doc: &Document) -> u32 {
    doc.objects.keys().map(|id| id.0).max().unwrap_or(0)
}

struct Staged {
    objects: Vec<(ObjectId, Object)>,
    pages: Vec<ObjectId>,
    next_id: u32,
}

/// Copy the pages of `source` (and everything they reach) into fresh object
/// numbers starting at `first_id`, parented to `parent`.
fn stage(source: &Source, first_id: u32, parent: ObjectId) -> Result<Staged, String> {
    let doc = &source.doc;
    let page_set: HashSet<ObjectId> = source.pages.iter().copied().collect();

    // Pages with their inherited attributes written in and no /Parent.
    let mut pages: HashMap<ObjectId, Dictionary> = HashMap::new();
    for &id in &source.pages {
        pages.insert(id, flatten_page(doc, id)?);
    }

    // Collect everything the pages reach, in a stable order.
    let mut order: Vec<ObjectId> = source.pages.clone();
    let mut seen: HashSet<ObjectId> = page_set.clone();
    let mut queue: VecDeque<ObjectId> = VecDeque::new();
    for id in &source.pages {
        collect_refs_dict(&pages[id], &mut |r| {
            if seen.insert(r) {
                queue.push_back(r);
            }
        });
    }
    while let Some(id) = queue.pop_front() {
        let Ok(object) = doc.get_object(id) else {
            continue; // Dangling reference: becomes null below.
        };
        order.push(id);
        let skip_parent = matches!(object.type_name(), Ok(b"Page" | b"Pages"));
        collect_refs(object, skip_parent, &mut |r| {
            if seen.insert(r) {
                queue.push_back(r);
            }
        });
    }

    let mut map: BTreeMap<ObjectId, ObjectId> = BTreeMap::new();
    let mut next = first_id;
    for &id in &order {
        map.insert(id, (next, 0));
        next = next
            .checked_add(1)
            .ok_or_else(|| "too many objects".to_string())?;
    }

    let mut objects = Vec::with_capacity(order.len());
    for &id in &order {
        let is_page = pages.contains_key(&id);
        let mut object = if let Some(dict) = pages.get(&id) {
            Object::Dictionary(dict.clone())
        } else {
            let mut object = doc.get_object(id).map_err(|e| e.to_string())?.clone();
            if !page_set.contains(&id) && matches!(object.type_name(), Ok(b"Page" | b"Pages")) {
                // A page-tree node reached through e.g. an annotation: keep it
                // detached so it cannot drag the source tree in.
                if let Ok(dict) = object.as_dict_mut() {
                    dict.remove(b"Parent");
                }
            }
            object
        };
        remap(&mut object, &map);
        if is_page && let Ok(dict) = object.as_dict_mut() {
            // Set after remapping: `parent` is a target-document id.
            dict.set("Parent", Object::Reference(parent));
        }
        objects.push((map[&id], object));
    }
    let new_pages = source.pages.iter().map(|id| map[id]).collect();
    Ok(Staged {
        objects,
        pages: new_pages,
        next_id: next,
    })
}

/// A copy of the page dictionary with every inheritable attribute present.
fn flatten_page(doc: &Document, id: ObjectId) -> Result<Dictionary, String> {
    let mut page = doc
        .get_dictionary(id)
        .map_err(|_| "page is not a dictionary".to_string())?
        .clone();
    let mut parent = page.get(b"Parent").and_then(Object::as_reference).ok();
    let mut depth = 0;
    let mut visited = HashSet::new();
    while let Some(parent_id) = parent {
        if depth >= MAX_TREE_DEPTH || !visited.insert(parent_id) {
            break;
        }
        depth += 1;
        let Ok(node) = doc.get_dictionary(parent_id) else {
            break;
        };
        for key in INHERITABLE {
            if !page.has(key)
                && let Ok(value) = node.get(key)
            {
                page.set(key.to_vec(), value.clone());
            }
        }
        parent = node.get(b"Parent").and_then(Object::as_reference).ok();
    }
    page.remove(b"Parent");
    // The source's structure tree is not copied.
    page.remove(b"StructParents");
    if !page.has(b"Resources") {
        page.set("Resources", Object::Dictionary(Dictionary::new()));
    }
    if !page.has(b"MediaBox") {
        page.set(
            "MediaBox",
            Object::Array(
                FALLBACK_MEDIA_BOX
                    .iter()
                    .map(|&v| Object::Integer(v))
                    .collect(),
            ),
        );
    }
    if !page.has(b"Rotate") {
        // Explicit, so the target's page tree cannot impose its own.
        page.set("Rotate", Object::Integer(0));
    }
    Ok(page)
}

fn collect_refs_dict(dict: &Dictionary, found: &mut impl FnMut(ObjectId)) {
    for (_, value) in dict {
        collect_refs(value, false, found);
    }
}

fn collect_refs(object: &Object, skip_parent: bool, found: &mut impl FnMut(ObjectId)) {
    match object {
        Object::Reference(id) => found(*id),
        Object::Array(items) => {
            for item in items {
                collect_refs(item, false, found);
            }
        }
        Object::Dictionary(dict) => {
            for (key, value) in dict {
                if !(skip_parent && key == b"Parent") {
                    collect_refs(value, false, found);
                }
            }
        }
        Object::Stream(stream) => {
            for (_, value) in &stream.dict {
                collect_refs(value, false, found);
            }
        }
        _ => {}
    }
}

fn remap(object: &mut Object, map: &BTreeMap<ObjectId, ObjectId>) {
    match object {
        Object::Reference(id) => {
            *object = map
                .get(id)
                .map_or(Object::Null, |&new| Object::Reference(new));
        }
        Object::Array(items) => {
            for item in items {
                remap(item, map);
            }
        }
        Object::Dictionary(dict) => remap_dict(dict, map),
        Object::Stream(stream) => remap_dict(&mut stream.dict, map),
        _ => {}
    }
}

fn remap_dict(dict: &mut Dictionary, map: &BTreeMap<ObjectId, ObjectId>) {
    for (_, value) in dict.iter_mut() {
        remap(value, map);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{Stream, dictionary};

    /// A PDF whose `n` pages sit under an intermediate /Pages node that
    /// carries the shared `Resources`, `MediaBox` and `Rotate` (all inherited).
    fn inherited_pdf(n: usize, media: [i64; 4], rotate: i64, label: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let root_id = doc.new_object_id();
        let mid_id = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
        let mut kids = Vec::new();
        for i in 0..n {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new("Td", vec![72.into(), 500.into()]),
                    Operation::new(
                        "Tj",
                        vec![Object::string_literal(format!("{label} {}", i + 1))],
                    ),
                    Operation::new("ET", vec![]),
                ],
            };
            let stream = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            kids.push(Object::Reference(doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => mid_id, "Contents" => stream,
            })));
        }
        doc.objects.insert(
            mid_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Parent" => root_id, "Kids" => kids,
                "Count" => i64::try_from(n).unwrap(),
                "Resources" => resources,
                "MediaBox" => media.iter().map(|&v| Object::Integer(v)).collect::<Vec<_>>(),
                "Rotate" => rotate,
            }),
        );
        doc.objects.insert(
            root_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![Object::Reference(mid_id)],
                "Count" => i64::try_from(n).unwrap(),
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root_id });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn page_dicts(pdf: &[u8]) -> Vec<Dictionary> {
        let doc = Document::load_mem(pdf).unwrap();
        doc.get_pages()
            .values()
            .map(|&id| doc.get_dictionary(id).unwrap().clone())
            .collect()
    }

    #[test]
    fn appends_pages_with_inherited_attributes() {
        let base = inherited_pdf(2, [0, 0, 595, 842], 0, "Base");
        let extra = inherited_pdf(3, [0, 0, 612, 1008], 90, "Extra");
        let mut merger = Merger::new(&base).unwrap();
        assert_eq!(merger.append(&extra).unwrap(), 3);
        assert_eq!(merger.page_count(), 5);
        let out = merger.to_pdf().unwrap();
        let pages = page_dicts(&out);
        assert_eq!(pages.len(), 5);
        let rotate = |d: &Dictionary| d.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
        assert_eq!(rotate(&pages[0]), 0);
        assert_eq!(rotate(&pages[2]), 90);
        let media = pages[4].get(b"MediaBox").unwrap().as_array().unwrap();
        assert_eq!(media[3].as_i64().unwrap(), 1008);
        let doc = Document::load_mem(&out).unwrap();
        let text = doc.extract_text(&[5]).unwrap();
        assert!(text.contains("Extra 3"), "{text}");
        assert!(
            doc.get_page_fonts(doc.get_pages()[&5])
                .unwrap()
                .contains_key(b"F1".as_slice())
        );
    }

    #[test]
    fn many_appends_keep_order() {
        let mut merger = Merger::new(&inherited_pdf(1, [0, 0, 595, 842], 0, "Doc0")).unwrap();
        for i in 1..=5 {
            merger
                .append(&inherited_pdf(2, [0, 0, 595, 842], 0, &format!("Doc{i}")))
                .unwrap();
        }
        let doc = Document::load_mem(&merger.to_pdf().unwrap()).unwrap();
        assert_eq!(doc.get_pages().len(), 11);
        assert!(doc.extract_text(&[11]).unwrap().contains("Doc5 2"));
        assert!(doc.extract_text(&[2]).unwrap().contains("Doc1 1"));
    }

    #[test]
    fn bad_input_is_rejected_and_state_is_kept() {
        let base = inherited_pdf(1, [0, 0, 595, 842], 0, "Base");
        let mut merger = Merger::new(&base).unwrap();
        let before = merger.to_pdf().unwrap();
        let good = inherited_pdf(4, [0, 0, 595, 842], 0, "Good");
        let cases: Vec<(&[u8], InvalidKind)> = vec![
            (b"", InvalidKind::Empty),
            (b"GIF89a not a pdf", InvalidKind::NotPdf),
            (b"%PDF-1.7\n garbage only", InvalidKind::Corrupt),
            (&good[..good.len() / 3], InvalidKind::Corrupt),
        ];
        for (input, kind) in cases {
            match merger.append(input) {
                Err(MergeError::Invalid(k, _)) => assert_eq!(k, kind),
                other => panic!("expected {kind:?}, got {other:?}"),
            }
            assert_eq!(merger.page_count(), 1);
            assert_eq!(merger.to_pdf().unwrap(), before);
        }
    }

    #[test]
    fn page_limit_is_enforced() {
        let mut merger = Merger::new(&inherited_pdf(2, [0, 0, 595, 842], 0, "B")).unwrap();
        merger.set_max_pages(4);
        let three = inherited_pdf(3, [0, 0, 595, 842], 0, "C");
        assert!(matches!(
            merger.append(&three),
            Err(MergeError::Limit(LimitKind::Pages, _))
        ));
        assert_eq!(merger.page_count(), 2);
        merger.set_max_input_bytes(100);
        assert!(matches!(
            merger.append(&three),
            Err(MergeError::Limit(LimitKind::InputBytes, _))
        ));
    }

    #[test]
    fn merge_all_reports_the_index() {
        let a = inherited_pdf(1, [0, 0, 595, 842], 0, "A");
        let err = merge_all(&[&a, &a, b"nope"]).unwrap_err();
        assert!(err.message().starts_with("PDF #2:"), "{err}");
        assert!(merge_all(&[]).is_err());
        let doc = Document::load_mem(&merge_all(&[&a, &a]).unwrap()).unwrap();
        assert_eq!(doc.get_pages().len(), 2);
    }
}
