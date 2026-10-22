//! Smart masks: masks as recipes, computed on each photo. A preset, a paste or a sync brings
//! masks whose AI selections (Subject, Sky, People by name or role, remembered objects, anchored
//! gradients…) have no segmentation for the photo yet; [`Session::compute_masks`] computes them on
//! the AI worker — in the app in the background, one activity row for the batch, the photo
//! showing the approximate mask meanwhile; in the CLI / MCP / tests waiting for them.
//!
//! The results never add undo steps of their own: they complete the step that brought the masks
//! (one undo step per preset application). Undoing that step while its masks are still computing
//! cancels the rest of the batch; redoing it computes what is still missing.
//!
//! Also here: the library's people and concept stores ([`crate::people_store`],
//! [`crate::concepts_store`]), loaded when first needed and saved on every change.
// (without the AI models — the CLI and web builds — the recipes stay approximate)
#![cfg_attr(not(feature = "sam"), allow(dead_code, unused_imports))]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_develop::recipe::{AnchorTo, Who};
use lightcraft_develop::{DevelopSettings, MaskShape, SegMask};
use serde_json::{Value, json};

use crate::Session;
use crate::concepts_store::ConceptStore;
use crate::people_store::{PeopleStore, PhotoContext};
use crate::segment::AutoKind;

/// What a recipe job computes for a component.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Part {
    /// The component's own segmentation.
    Seg(AutoKind),
    /// The box an anchored gradient is placed in.
    Anchor,
    /// An Object selection from its clicks.
    Object,
    /// A Describe selection from its text.
    Prompt,
}

/// A batch of recipe jobs (one preset application, paste, sync, import or update).
struct Batch {
    photos: HashSet<PhotoId>,
    pending: usize,
    total: usize,
    guard: Option<crate::activity::TaskGuard>,
    notes: BTreeSet<String>,
}

impl SmartState {
    /// Whether AI masks are still being computed for any of `ids` (a batch with work pending).
    pub(crate) fn busy_with(&self, ids: &HashSet<PhotoId>) -> bool {
        self.batches.values().any(|b| b.pending > 0 && b.photos.iter().any(|p| ids.contains(p)))
    }
}

/// The smart-mask state of a session.
#[derive(Default)]
pub struct SmartState {
    next_batch: u64,
    batches: HashMap<u64, Batch>,
    /// Batches cancelled (the worker skips their queued jobs).
    pub(crate) cancelled: Arc<Mutex<HashSet<u64>>>,
    /// The library the stores were loaded from (`Some(None)`: none, in memory).
    loaded_for: Option<Option<std::path::PathBuf>>,
    people: PeopleStore,
    concepts: ConceptStore,
    /// Faces found per photo (`people.faces`): the photo's segment key and the faces.
    #[cfg(feature = "sam")]
    pub(crate) faces: HashMap<PhotoId, (u64, Vec<crate::segment::worker::FoundFace>)>,
    /// Photos whose faces are being looked for.
    pub(crate) faces_pending: HashSet<PhotoId>,
    /// Notes for the user from finished work (the UI shows them; the CLI returns them).
    pub(crate) messages: Vec<String>,
    /// Hover preview selections: (photo, its look's key, recipe key) → selection.
    preview: HashMap<(PhotoId, u64, u64), SegMask>,
    pub(crate) preview_pending: HashSet<(PhotoId, u64, u64)>,
    /// The batch of the preview jobs in flight (0: none).
    preview_batch: u64,
    /// A store that couldn't be read, once per library (kept from being overwritten).
    blocked: HashSet<&'static str>,
}

/// A recipe that couldn't be computed, in words for the photographer.
fn missing_note(kind: &AutoKind) -> String {
    match kind {
        AutoKind::People { who: Some(w), .. } => format!("{} wasn't recognised on this photo", w.label()),
        AutoKind::People { .. } => "No people were found in this photo".into(),
        AutoKind::Concept { name } => format!("“{name}” wasn't found on this photo"),
        AutoKind::Sky => "No sky was found in this photo".into(),
        _ => "Nothing of that kind was found in this photo".into(),
    }
}

/// The box (normalized) around what `seg` selects; `None` when it selects nothing.
pub fn seg_box(seg: &SegMask) -> Option<[f64; 4]> {
    let logits = seg.logits()?;
    let r = seg.bounds()?;
    let side = seg.side as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (i, l) in logits.iter().enumerate() {
        if *l > 0.0 {
            let (x, y) = (i % side.max(1), i / side.max(1));
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    if x0 == usize::MAX {
        return None;
    }
    let (rw, rh) = (r[2] - r[0], r[3] - r[1]);
    let s = side.max(1) as f64;
    lightcraft_develop::recipe::valid_box([
        r[0] + x0 as f64 / s * rw,
        r[1] + y0 as f64 / s * rh,
        r[0] + x1 as f64 / s * rw,
        r[1] + y1 as f64 / s * rh,
    ])
}

/// How a person named by a recipe is found on one photo.
pub(crate) enum Resolved {
    /// Their face box here (a named face region) and/or their known appearance.
    Person { rect: Option<[f64; 4]>, known: Vec<Value> },
    /// Nobody has the role here: the mask covers everyone (and says so).
    Everyone(String),
    /// Nothing to find them by.
    Missing(String),
}

/// Width / height of photo `p` as masks see it (oriented); 3:2 when unknown.
pub fn aspect_of(p: &lightcraft_catalog::Photo) -> f64 {
    if p.width == 0 || p.height == 0 {
        return lightcraft_develop::recipe::RECIPE_ASPECT;
    }
    let (w, h) = (f64::from(p.width), f64::from(p.height));
    let a = if p.develop.orientation.swaps_axes() { h / w } else { w / h };
    if a.is_finite() && a > 0.0 { a } else { lightcraft_develop::recipe::RECIPE_ASPECT }
}

impl Session {
    /// Width / height of photo `id` as masks see it (oriented); 3:2 when unknown.
    pub fn photo_aspect(&self, id: PhotoId) -> f64 {
        self.catalog.photo(id).map_or(lightcraft_develop::recipe::RECIPE_ASPECT, |p| aspect_of(p))
    }

    /// Remember an example of a remembered object (`concepts.teach`). → its examples.
    #[cfg(feature = "sam")]
    pub(crate) fn store_concept_example(&mut self, name: &str, photo: PhotoId, ex: lightcraft_segment::concept::Example) -> Result<usize, String> {
        let v = serde_json::to_value(ex).map_err(|e| e.to_string())?;
        self.with_concepts(|c| c.teach(name, photo.0, v))?
    }

    /// What a photo's scope (for roles) is judged by.
    pub fn photo_context(&self, id: PhotoId) -> PhotoContext {
        let Some(p) = self.catalog.photo(id) else { return PhotoContext { id: id.0, ..Default::default() } };
        let folder = match &p.source {
            lightcraft_catalog::Source::File { path } => {
                let n = crate::people_store::norm_folder(path);
                n.rsplit_once('/').map(|(dir, _)| dir.to_string())
            }
            _ => None,
        };
        PhotoContext { id: id.0, folder, albums: self.catalog.albums_of(id).into_iter().map(|a| a.0).collect(), date: p.date().to_string() }
    }

    /// Load the stores for the open library (once per library).
    fn smart_load(&mut self) {
        let dir = self.library.as_ref().map(|l| l.dir.clone());
        if self.segmenter.smart.loaded_for.as_ref() == Some(&dir) {
            return;
        }
        let st = &mut self.segmenter.smart;
        st.loaded_for = Some(dir);
        st.people = PeopleStore::default();
        st.concepts = ConceptStore::default();
        st.blocked.clear();
        let Some(lib) = self.library.as_mut() else { return };
        for file in [crate::people_store::FILE, crate::concepts_store::FILE] {
            match lib.read_file(file) {
                Ok(None) => {}
                Ok(Some(bytes)) => {
                    let parsed = if file == crate::people_store::FILE {
                        PeopleStore::from_bytes(&bytes).map(|s| self.segmenter.smart.people = s)
                    } else {
                        ConceptStore::from_bytes(&bytes).map(|s| self.segmenter.smart.concepts = s)
                    };
                    if let Err(e) = parsed {
                        log::error!("{e}");
                        self.segmenter.smart.messages.push(format!("{e}. It was left as it is; LightCraft starts with an empty list."));
                        self.segmenter.smart.blocked.insert(file);
                    }
                }
                Err(e) => log::error!("{file}: {e}"),
            }
        }
    }

    /// The people store (roles, appearance examples).
    pub fn people_store(&mut self) -> &PeopleStore {
        self.smart_load();
        &self.segmenter.smart.people
    }

    /// Change the people store and save it.
    pub fn with_people<T>(&mut self, f: impl FnOnce(&mut PeopleStore) -> T) -> Result<T, String> {
        self.smart_load();
        let out = f(&mut self.segmenter.smart.people);
        self.smart_save(crate::people_store::FILE)?;
        Ok(out)
    }

    /// The concept store.
    pub fn concept_store(&mut self) -> &ConceptStore {
        self.smart_load();
        &self.segmenter.smart.concepts
    }

    /// Change the concept store and save it.
    pub fn with_concepts<T>(&mut self, f: impl FnOnce(&mut ConceptStore) -> T) -> Result<T, String> {
        self.smart_load();
        let out = f(&mut self.segmenter.smart.concepts);
        self.smart_save(crate::concepts_store::FILE)?;
        Ok(out)
    }

    fn smart_save(&mut self, file: &'static str) -> Result<(), String> {
        if self.segmenter.smart.blocked.contains(file) {
            return Err(format!("{file} couldn't be read, so it isn't overwritten; move it aside to start a new one"));
        }
        let bytes = if file == crate::people_store::FILE { self.segmenter.smart.people.to_bytes() } else { self.segmenter.smart.concepts.to_bytes() };
        match self.library.as_mut() {
            Some(lib) => lib.write_file(file, &bytes).map_err(|e| format!("couldn't save {file}: {e}")),
            None => Ok(()),
        }
    }

    /// Who `who` is on photo `id`.
    pub(crate) fn resolve_who(&mut self, id: PhotoId, who: &Who) -> Resolved {
        let ctx = self.photo_context(id);
        let name = match who {
            Who::Name(n) => n.trim().to_string(),
            Who::Role(r) => match self.people_store().role_for(r, &ctx) {
                Some(n) => n.to_string(),
                None => {
                    return Resolved::Everyone(format!(
                        "No {} is set for this shoot, so the mask covers everyone (set it in the People picker: Roles…)",
                        r.trim()
                    ));
                }
            },
        };
        let rect = self.catalog.photo(id).and_then(|p| {
            p.meta
                .regions
                .iter()
                .find(|r| r.kind == lightcraft_meta::RegionKind::Face && r.name.as_deref().is_some_and(|n| n.trim().eq_ignore_ascii_case(&name)))
                .map(|r| [r.rect.x0, r.rect.y0, r.rect.x1, r.rect.y1])
        });
        // their examples from this shoot when there are some (appearance holds within a shoot)
        let examples: Vec<crate::people_store::Example> = self.people_store().person(&name).map(|p| p.examples.clone()).unwrap_or_default();
        let here: Vec<Value> = examples
            .iter()
            .filter(|e| ctx.folder.is_some() && self.photo_context(PhotoId(e.photo)).folder == ctx.folder)
            .map(|e| e.signature.clone())
            .collect();
        let known: Vec<Value> = if here.is_empty() { examples.into_iter().map(|e| e.signature).collect() } else { here };
        if rect.is_none() && known.is_empty() {
            return Resolved::Missing(format!("{name} has no named face yet: name them in the People picker of the Masking panel"));
        }
        Resolved::Person { rect, known }
    }

    /// The automatic name of mask `m` on photo `id`: what it selects, People with their role and
    /// the name it has in this shoot ("Groom Jamie - Clothes, Hair").
    pub fn auto_mask_name(&mut self, id: PhotoId, m: &lightcraft_develop::Mask) -> String {
        let mut m = m.clone();
        let ctx = self.photo_context(id);
        for c in &mut m.components {
            if let MaskShape::People { who: Some(Who::Role(r)), .. } = &mut c.shape
                && let Some(n) = self.people_store().role_for(r, &ctx).map(str::to_string)
            {
                *r = format!("{} {n}", r.trim());
            }
        }
        lightcraft_develop::recipe::content_name(&m)
    }

    /// Give the masks of `after` that still carry an automatic name (a placeholder, or the name
    /// they had automatically in `before`) the name of what they select now.
    pub(crate) fn refresh_mask_names(&mut self, id: PhotoId, before: &[lightcraft_develop::Mask], after: &mut [lightcraft_develop::Mask]) {
        for m in after.iter_mut() {
            let was = before.iter().find(|b| b.id == m.id).map(|b| self.auto_mask_name(id, b));
            if lightcraft_develop::recipe::generic_name(&m.name) || was.as_deref() == Some(m.name.as_str()) {
                if m.components.is_empty() {
                    continue;
                }
                m.name = self.auto_mask_name(id, m);
            }
        }
    }

    /// Change photo `id`'s settings as the completion of the step that is already in the history
    /// (no undo step of its own): saved, and undone with that step.
    pub(crate) fn amend_develop(&mut self, id: PhotoId, settings: DevelopSettings) {
        let op = Op::SetDevelop { id, settings: Arc::new(settings), label: "AI masks".into(), edited: Some((self.clock)()) };
        match self.catalog.apply(op.clone()) {
            Ok(_) => self.pending_log.push(op),
            Err(e) => log::warn!("AI masks: {e}"),
        }
    }

    /// Compute the AI parts of every mask on `ids` that needs it (`force`: recompute all of them):
    /// queued in the background in the app, computed before returning otherwise. →
    /// `{queued, updated, approximate, notes, batch}`.
    pub fn compute_masks(&mut self, ids: &[PhotoId], force: bool) -> Value {
        let mut notes: BTreeSet<String> = BTreeSet::new();
        let mut jobs: Vec<(PhotoId, u32, usize, Part)> = Vec::new();
        for id in ids {
            let Some(mut d) = self.develop_of(*id).map(|d| (*d).clone()) else { continue };
            let aspect = self.photo_aspect(*id);
            if force {
                // recompute everything: back to the recipe, the approximate mask shows meanwhile
                let mut changed = false;
                for m in &mut d.masks {
                    if m.uses_ai() {
                        m.strip_ai(aspect);
                        changed = true;
                    }
                }
                if changed {
                    self.amend_develop(*id, d.clone());
                }
            }
            // what the hover preview computed already (the preset was looked at before it was
            // clicked) goes straight in
            let key = self.segment_input_key(*id).unwrap_or(0);
            let mut filled = false;
            for m in &mut d.masks {
                if m.components
                    .iter()
                    .any(|c| c.shape.is_ai() && !c.shape.has_seg() && AutoKind::of(&c.shape).is_none() && matches!(c.shape, MaskShape::People { .. }))
                {
                    notes.insert("Some People parts have no model and keep an approximate mask".into());
                }
                for (k, part) in parts_of(m) {
                    let Some(c) = m.components.get_mut(k) else { continue };
                    let pk = part_key(c, &part);
                    if let Some(seg) = self.segmenter.smart.preview.get(&(*id, key, pk)).cloned()
                        && fill_part(c, &part, seg, aspect)
                    {
                        filled = true;
                        continue;
                    }
                    jobs.push((*id, m.id, k, part));
                }
            }
            if filled {
                self.amend_develop(*id, d);
            }
        }
        if jobs.is_empty() {
            return json!({"queued": 0, "updated": 0, "notes": notes});
        }
        #[cfg(not(feature = "sam"))]
        {
            notes.insert("AI masks are not available in this build: the masks are approximate".into());
            json!({"queued": 0, "updated": 0, "approximate": jobs.len(), "notes": notes})
        }
        #[cfg(feature = "sam")]
        self.queue_recipe_jobs(jobs, notes)
    }

    #[cfg(feature = "sam")]
    fn queue_recipe_jobs(&mut self, jobs: Vec<(PhotoId, u32, usize, Part)>, mut notes: BTreeSet<String>) -> Value {
        let background = self.segmenter.background;
        let batch = {
            let st = &mut self.segmenter.smart;
            st.next_batch = st.next_batch.wrapping_add(1);
            st.next_batch
        };
        let photos: HashSet<PhotoId> = jobs.iter().map(|j| j.0).collect();
        let mut queued = 0usize;
        let mut approximate = 0usize;
        let mut updated = 0usize;
        let mut missing_models: BTreeSet<String> = BTreeSet::new();
        let mut submitted: Vec<(PhotoId, crate::segment::worker::Kind, crate::segment::Tag)> = Vec::new();
        for (photo, mask, comp, part) in jobs {
            match self.recipe_kind(photo, mask, comp, &part) {
                Ok(Some(kind)) => {
                    let tag = crate::segment::Tag::Recipe { photo, mask, comp, part, batch };
                    submitted.push((photo, kind, tag));
                }
                Ok(None) => {}
                Err(RecipeSkip::Note(n)) => {
                    notes.insert(n);
                }
                Err(RecipeSkip::Model(m)) => {
                    approximate += 1;
                    missing_models.insert(m);
                }
            }
        }
        for m in missing_models {
            notes.insert(format!("Some masks are approximate until the {m} model is installed (Masking panel: Get the AI Model)"));
        }
        if background && !submitted.is_empty() {
            // one batch row (client note 34): masks done and failed, the time left; each photo the worker analyzes
            // shows under it
            let guard = self.activity.start_batch("ai", "Updating AI masks", crate::activity::Cancel::Yes);
            guard.progress(0, submitted.len() as u64);
            self.segmenter
                .smart
                .batches
                .insert(batch, Batch { photos, pending: 0, total: submitted.len(), guard: Some(guard), notes: BTreeSet::new() });
        }
        for (photo, kind, tag) in submitted {
            if background {
                match self.segment_queue_batch(photo, kind, tag, batch) {
                    Ok(()) => {
                        queued += 1;
                        if let Some(b) = self.segmenter.smart.batches.get_mut(&batch) {
                            b.pending += 1;
                        }
                    }
                    Err(e) => {
                        notes.insert(e);
                    }
                }
            } else {
                match self.segment_wait_kind(photo, kind) {
                    Ok(result) => {
                        let mut polled = crate::segment::Polled::default();
                        let o = crate::segment::worker::Outcome {
                            tag,
                            result: Ok(result),
                            superseded: false,
                            faces: None,
                            payload: Default::default(),
                            note: None,
                            notice: None,
                        };
                        if self.apply_recipe_outcome(o, &mut polled) {
                            updated += 1;
                        }
                        notes.extend(polled.messages);
                    }
                    Err(e) => {
                        notes.insert(e);
                    }
                }
            }
        }
        if background {
            // nothing went out after all: no row
            if self.segmenter.smart.batches.get(&batch).is_some_and(|b| b.pending == 0) {
                self.segmenter.smart.batches.remove(&batch);
            }
            if let Some(b) = self.segmenter.smart.batches.get_mut(&batch) {
                b.total = b.pending;
                if let Some(g) = &b.guard {
                    g.progress(0, b.total as u64);
                }
            }
            self.segmenter.smart.messages.extend(notes.iter().cloned());
        }
        json!({"queued": queued, "updated": updated, "approximate": approximate, "notes": notes, "batch": batch})
    }

    /// The worker request for one recipe part.
    #[cfg(feature = "sam")]
    fn recipe_kind(&mut self, photo: PhotoId, mask: u32, comp: usize, part: &Part) -> Result<Option<crate::segment::worker::Kind>, RecipeSkip> {
        let d = self.develop_of(photo).ok_or(RecipeSkip::Note("no such photo".into()))?;
        let c = d.masks.iter().find(|m| m.id == mask).and_then(|m| m.components.get(comp)).ok_or(RecipeSkip::Note("no such mask".into()))?.clone();
        self.recipe_kind_for(photo, &c, part)
    }

    /// The worker request for one recipe part of component `c` on `photo`.
    #[cfg(feature = "sam")]
    fn recipe_kind_for(
        &mut self,
        photo: PhotoId,
        c: &lightcraft_develop::MaskComponent,
        part: &Part,
    ) -> Result<Option<crate::segment::worker::Kind>, RecipeSkip> {
        use crate::segment::worker::{Kind, Target};
        let auto = |s: &mut Session, what: AutoKind, target: Target| -> Result<Option<Kind>, RecipeSkip> {
            match s.segmenter.auto_model(&what) {
                Ok(model) => Ok(Some(Kind::Auto { what, model, target })),
                Err(_) => Err(RecipeSkip::Model(model_title(what.preferred_model()))),
            }
        };
        match part {
            Part::Seg(AutoKind::People { person, parts, who: Some(who) }) => match self.resolve_who(photo, who) {
                Resolved::Person { rect, known } => {
                    let known = known.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect();
                    let sam2 = self.segmenter.require_model("sam2").ok();
                    auto(
                        self,
                        AutoKind::People { person: *person, parts: parts.clone(), who: Some(who.clone()) },
                        Target::Person { rect, known, sam2 },
                    )
                }
                Resolved::Everyone(note) => {
                    self.note_batchless(note);
                    auto(self, AutoKind::People { person: 0, parts: parts.clone(), who: None }, Target::None)
                }
                Resolved::Missing(note) => Err(RecipeSkip::Note(note)),
            },
            Part::Seg(AutoKind::Concept { name }) => {
                let examples = self.concept_examples(name);
                if examples.is_empty() {
                    return Err(RecipeSkip::Note(format!("No remembered object is called “{name}” (Masking panel: Remember Selected Mask As…)")));
                }
                auto(self, AutoKind::Concept { name: name.clone() }, Target::Concept(Arc::new(examples)))
            }
            Part::Seg(kind) => auto(self, kind.clone(), Target::None),
            Part::Anchor => {
                let Some(a) = &c.anchor else { return Ok(None) };
                let what = match &a.to {
                    AnchorTo::Subject => AutoKind::Subject,
                    AnchorTo::People => AutoKind::People { person: 0, parts: Vec::new(), who: None },
                    AnchorTo::Who(w) => AutoKind::People { person: 0, parts: Vec::new(), who: Some(w.clone()) },
                };
                let target = match &what {
                    AutoKind::People { who: Some(w), .. } => match self.resolve_who(photo, w) {
                        Resolved::Person { rect, known } => Target::Person {
                            rect,
                            known: known.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect(),
                            sam2: self.segmenter.require_model("sam2").ok(),
                        },
                        Resolved::Everyone(_) => return auto(self, AutoKind::People { person: 0, parts: Vec::new(), who: None }, Target::None),
                        Resolved::Missing(n) => return Err(RecipeSkip::Note(n)),
                    },
                    _ => Target::None,
                };
                auto(self, what, target)
            }
            Part::Object => {
                let MaskShape::Object { hint, exclude, .. } = &c.shape else { return Ok(None) };
                if self.segmenter.object_model().is_err() {
                    return Err(RecipeSkip::Model("SAM 2.1".into()));
                }
                let clicks: Vec<lightcraft_segment::Click> = hint
                    .iter()
                    .map(|p| (p, true))
                    .chain(exclude.iter().map(|p| (p, false)))
                    .take(crate::segment::MAX_CLICKS)
                    .map(|(p, positive)| lightcraft_segment::Click { x: p.x as f32, y: p.y as f32, positive })
                    .collect();
                Ok(Some(Kind::Clicks(clicks)))
            }
            Part::Prompt => {
                let MaskShape::Prompt { text, .. } = &c.shape else { return Ok(None) };
                if self.segmenter.describe_model().is_err() {
                    return Err(RecipeSkip::Model(match self.segmenter.describe_needs().first() {
                        Some(key) => model_title(key),
                        None => "SAM 3".into(),
                    }));
                }
                Ok(Some(Kind::Text(text.clone())))
            }
        }
    }

    fn note_batchless(&mut self, note: String) {
        if !self.segmenter.smart.messages.contains(&note) {
            self.segmenter.smart.messages.push(note);
        }
    }

    /// A remembered object's examples, parsed.
    #[cfg(feature = "sam")]
    pub(crate) fn concept_examples(&mut self, name: &str) -> Vec<lightcraft_segment::concept::Example> {
        self.concept_store().get(name).map(|c| c.examples.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect()).unwrap_or_default()
    }

    /// Apply a finished recipe job (from the worker or a waiting call); true when a mask changed.
    #[cfg(feature = "sam")]
    pub(crate) fn apply_recipe_outcome(&mut self, o: crate::segment::worker::Outcome, polled: &mut crate::segment::Polled) -> bool {
        let crate::segment::Tag::Recipe { photo, mask, comp, part, batch } = o.tag else { return false };
        let finished = self.finish_batch_job(batch, o.superseded);
        if let Some(note) = o.note.clone().filter(|_| !o.superseded) {
            self.batch_note(batch, note, polled);
        }
        let changed = (|| {
            if o.superseded {
                return false;
            }
            let seg = match o.result {
                Ok(Some(seg)) => seg,
                Ok(None) => {
                    let note = match &part {
                        Part::Seg(k) => missing_note(k),
                        Part::Anchor => "The gradient's subject wasn't found, so it spans the whole photo".to_string(),
                        _ => "Nothing was selected on this photo".to_string(),
                    };
                    self.batch_note(batch, note, polled);
                    // settled: nothing there (a gradient: the whole photo), not asked again on every
                    // later preset (Update AI Masks with `force` asks again)
                    SegMask::from_logits(1, &[if part == Part::Anchor { 8.0 } else { -8.0 }])
                }
                Err(e) => {
                    if let Some(g) = self.segmenter.smart.batches.get(&batch).and_then(|b| b.guard.as_ref()) {
                        let name = self.catalog.photo(photo).map_or("", |p| p.file_name.as_str());
                        g.count_failed(&format!("{name}: {e}"));
                    }
                    self.batch_note(batch, e, polled);
                    return false;
                }
            };
            let aspect = self.photo_aspect(photo);
            let Some(d) = self.develop_of(photo) else { return false };
            let mut d = (*d).clone();
            let Some(c) = d.masks.iter_mut().find(|m| m.id == mask).and_then(|m| m.components.get_mut(comp)) else { return false };
            // only into the selection it was computed for, if nothing filled it meanwhile
            let fresh = match &part {
                Part::Anchor => c.anchor.as_ref().is_some_and(|a| a.bounds.is_none()),
                _ => !c.shape.has_seg(),
            };
            let ok = fresh && fill_part(c, &part, seg, aspect);
            if ok {
                self.amend_develop(photo, d);
                polled.changed = true;
            }
            ok
        })();
        if finished {
            self.close_batch(batch, polled);
        }
        changed
    }

    /// The activity row of recipe batch `batch`, if it has one (the worker's per-photo rows go under it).
    #[cfg_attr(not(feature = "sam"), allow(dead_code))]
    pub(crate) fn smart_batch_row(&self, batch: u64) -> Option<u64> {
        self.segmenter.smart.batches.get(&batch).and_then(|b| b.guard.as_ref()).map(|g| g.id())
    }

    /// Count a job of `batch` done; true when it was the last.
    fn finish_batch_job(&mut self, batch: u64, _superseded: bool) -> bool {
        let Some(b) = self.segmenter.smart.batches.get_mut(&batch) else { return false };
        b.pending = b.pending.saturating_sub(1);
        if let Some(g) = &b.guard {
            g.progress((b.total - b.pending.min(b.total)) as u64, b.total as u64);
        }
        b.pending == 0
    }

    fn batch_note(&mut self, batch: u64, note: String, polled: &mut crate::segment::Polled) {
        match self.segmenter.smart.batches.get_mut(&batch) {
            Some(b) => {
                b.notes.insert(note);
            }
            None => {
                if !polled.messages.contains(&note) {
                    polled.messages.push(note);
                }
            }
        }
    }

    fn close_batch(&mut self, batch: u64, polled: &mut crate::segment::Polled) {
        if let Some(b) = self.segmenter.smart.batches.remove(&batch) {
            for n in b.notes {
                if !polled.messages.contains(&n) {
                    polled.messages.push(n);
                }
            }
        }
    }

    /// Cancel the batches still computing for any of `photos` (their step was undone); the
    /// worker skips their queued jobs.
    pub(crate) fn cancel_batches_for(&mut self, photos: &[PhotoId]) -> usize {
        let ids: Vec<u64> =
            self.segmenter.smart.batches.iter().filter(|(_, b)| b.photos.iter().any(|p| photos.contains(p))).map(|(k, _)| *k).collect();
        let mut set = self.segmenter.smart.cancelled.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for id in &ids {
            set.insert(*id);
        }
        drop(set);
        for id in &ids {
            self.segmenter.smart.batches.remove(id);
        }
        ids.len()
    }

    /// Batches whose activity row the user cancelled (✕).
    pub(crate) fn poll_batch_cancels(&mut self) {
        let ids: Vec<u64> =
            self.segmenter.smart.batches.iter().filter(|(_, b)| b.guard.as_ref().is_some_and(|g| g.is_cancelled())).map(|(k, _)| *k).collect();
        if ids.is_empty() {
            return;
        }
        let mut set = self.segmenter.smart.cancelled.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for id in &ids {
            set.insert(*id);
        }
        drop(set);
        for id in ids {
            self.segmenter.smart.batches.remove(&id);
        }
    }

    /// Whether AI masks are being computed for a batch.
    pub fn smart_busy(&self) -> bool {
        !self.segmenter.smart.batches.is_empty()
    }

    /// After an undo or redo of `op`: cancel what was computing for its photos, and compute the
    /// masks a preset application left uncomputed (redo, or an undo back to such a step).
    pub(crate) fn after_history_jump(&mut self, op: &Op) {
        let mut photos = Vec::new();
        collect_photos(op, &mut photos, 0);
        if photos.is_empty() {
            return;
        }
        self.cancel_batches_for(&photos);
        let todo: Vec<PhotoId> = photos
            .into_iter()
            .filter(|id| self.develop_of(*id).is_some_and(|d| d.masks.iter().any(|m| m.preset.is_some() && m.needs_compute())))
            .collect();
        if !todo.is_empty() {
            self.compute_masks(&todo, false);
        }
    }

    /// Notes from background smart-mask work, for the user.
    pub fn take_smart_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.segmenter.smart.messages)
    }
}

/// What a recipe part computes, as a key (its recipe; the person, role or object it names).
fn part_key(c: &lightcraft_develop::MaskComponent, part: &Part) -> u64 {
    let what = match part {
        Part::Anchor => serde_json::to_string(&c.anchor.as_ref().map(|a| &a.to)).unwrap_or_default(),
        _ => serde_json::to_string(&c.shape.recipe()).unwrap_or_default(),
    };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in what.bytes().chain([matches!(part, Part::Anchor) as u8]) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The parts of a mask that need computing, with their component index.
fn parts_of(m: &lightcraft_develop::Mask) -> Vec<(usize, Part)> {
    let mut out = Vec::new();
    for (k, c) in m.components.iter().enumerate() {
        if c.anchor.as_ref().is_some_and(|a| a.bounds.is_none()) {
            out.push((k, Part::Anchor));
        }
        if c.shape.has_seg() || !c.shape.is_ai() {
            continue;
        }
        let part = match &c.shape {
            MaskShape::Object { hint, .. } if !hint.is_empty() => Part::Object,
            MaskShape::Prompt { text, .. } if !text.trim().is_empty() => Part::Prompt,
            MaskShape::Object { .. } | MaskShape::Prompt { .. } => continue,
            shape => match AutoKind::of(shape) {
                Some(kind) => Part::Seg(kind),
                None => continue,
            },
        };
        out.push((k, part));
    }
    out
}

/// Put a computed `seg` into component `c` for `part` (anchors: its box). Whether it fit.
fn fill_part(c: &mut lightcraft_develop::MaskComponent, part: &Part, seg: SegMask, aspect: f64) -> bool {
    match part {
        Part::Seg(kind) => kind.seg_of(&c.shape).is_some_and(Option::is_none) && kind.set_seg(&mut c.shape, Some(seg)),
        Part::Anchor => match (&mut c.anchor, seg_box(&seg)) {
            (Some(a), Some(b)) => {
                a.bounds = Some(b);
                if let Some(s) = a.placed(aspect) {
                    c.shape = s;
                }
                true
            }
            _ => false,
        },
        Part::Object => match &mut c.shape {
            MaskShape::Object { seg: s @ None, .. } => {
                *s = Some(seg);
                true
            }
            _ => false,
        },
        Part::Prompt => match &mut c.shape {
            MaskShape::Prompt { seg: s @ None, .. } => {
                *s = Some(seg);
                true
            }
            _ => false,
        },
    }
}

/// Most selections the hover preview keeps (per photo look).
const PREVIEW_CACHE: usize = 64;

impl Session {
    /// A preset's look on photo `id` for the hover preview (`d`: the photo's settings with the
    /// preset applied), without changing anything: its masks use selections already known —
    /// computed for an earlier preview, or the same recipe already on the photo — and the masks
    /// still missing one are left out (the global part shows at once). With `compute`, the
    /// missing selections are computed in the background ([`Session::segment_poll`] reports
    /// them; ask again). → (the look, whether every mask is in it).
    pub fn preview_preset(&mut self, id: PhotoId, mut d: DevelopSettings, compute: bool) -> (DevelopSettings, bool) {
        let aspect = self.photo_aspect(id);
        let key = self.segment_input_key(id).unwrap_or(0);
        let own = self.develop_of(id);
        let mut complete = true;
        let mut wanted: Vec<(u64, lightcraft_develop::MaskComponent, Part)> = Vec::new();
        d.masks.retain_mut(|m| {
            let parts = parts_of(m);
            if parts.is_empty() {
                return true;
            }
            let mut ok = true;
            for (k, part) in parts {
                let Some(c) = m.components.get(k).cloned() else { continue };
                let pk = part_key(&c, &part);
                // the same selection already on the photo (its own mask) or computed for a preview
                let known = self.segmenter.smart.preview.get(&(id, key, pk)).cloned().or_else(|| {
                    own.as_ref().and_then(|o| {
                        o.masks
                            .iter()
                            .flat_map(|om| &om.components)
                            .find_map(|oc| (part_key(oc, &part) == pk && !matches!(part, Part::Anchor)).then(|| seg_of_shape(&oc.shape)).flatten())
                    })
                });
                let fitted = match (known, m.components.get_mut(k)) {
                    (Some(seg), Some(cm)) => fill_part(cm, &part, seg, aspect),
                    _ => false,
                };
                if !fitted {
                    ok = false;
                    wanted.push((pk, c, part));
                }
            }
            complete &= ok;
            ok
        });
        #[cfg(feature = "sam")]
        if compute && self.segmenter.background {
            for (pk, c, part) in wanted {
                if !self.segmenter.smart.preview_pending.insert((id, key, pk)) {
                    continue;
                }
                let batch = self.preview_batch();
                if let Ok(Some(kind)) = self.recipe_kind_for(id, &c, &part) {
                    let tag = crate::segment::Tag::Preview { photo: id, key, part_key: pk };
                    if self.segment_queue_batch(id, kind, tag, batch).is_err() {
                        self.segmenter.smart.preview_pending.remove(&(id, key, pk));
                    }
                } else {
                    self.segmenter.smart.preview_pending.remove(&(id, key, pk));
                }
            }
        }
        #[cfg(not(feature = "sam"))]
        let _ = (compute, wanted);
        (d, complete)
    }

    /// The batch the hover preview's jobs belong to (a new one after each cancel).
    #[cfg(feature = "sam")]
    fn preview_batch(&mut self) -> u64 {
        let st = &mut self.segmenter.smart;
        if st.preview_batch == 0 {
            st.next_batch = st.next_batch.wrapping_add(1);
            st.preview_batch = st.next_batch;
        }
        st.preview_batch
    }

    /// The pointer left the presets: skip the preview selections still queued.
    pub fn cancel_preview(&mut self) {
        let st = &mut self.segmenter.smart;
        if st.preview_batch != 0 {
            st.cancelled.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(st.preview_batch);
            st.preview_batch = 0;
            st.preview_pending.clear();
        }
    }

    /// Whether preview selections are being computed.
    pub fn preview_busy(&self) -> bool {
        !self.segmenter.smart.preview_pending.is_empty()
    }

    /// Store a preview selection (from the worker).
    #[cfg(feature = "sam")]
    pub(crate) fn apply_preview_outcome(&mut self, o: crate::segment::worker::Outcome, polled: &mut crate::segment::Polled) {
        let crate::segment::Tag::Preview { photo, key, part_key } = o.tag else { return };
        let st = &mut self.segmenter.smart;
        st.preview_pending.remove(&(photo, key, part_key));
        if let (false, Ok(Some(seg))) = (o.superseded, o.result) {
            if st.preview.len() >= PREVIEW_CACHE {
                st.preview.retain(|(p, k, _), _| *p == photo && *k == key);
                if st.preview.len() >= PREVIEW_CACHE {
                    st.preview.clear();
                }
            }
            st.preview.insert((photo, key, part_key), seg);
            polled.changed = true;
        }
    }
}

/// The computed selection a shape holds.
fn seg_of_shape(s: &MaskShape) -> Option<SegMask> {
    match s {
        MaskShape::Subject { seg }
        | MaskShape::Sky { seg }
        | MaskShape::Background { seg }
        | MaskShape::People { seg, .. }
        | MaskShape::Landscape { seg, .. }
        | MaskShape::Concept { seg, .. }
        | MaskShape::Object { seg, .. }
        | MaskShape::Prompt { seg, .. } => seg.clone(),
        _ => None,
    }
}

/// The photos an op changes the develop settings of.
fn collect_photos(op: &Op, out: &mut Vec<PhotoId>, depth: usize) {
    if depth > 8 {
        return;
    }
    match op {
        Op::SetDevelop { id, .. } => {
            if !out.contains(id) {
                out.push(*id);
            }
        }
        Op::Batch { ops } => ops.iter().for_each(|o| collect_photos(o, out, depth + 1)),
        _ => {}
    }
}

/// Why a recipe part wasn't queued.
#[cfg_attr(not(feature = "sam"), allow(dead_code))]
enum RecipeSkip {
    Note(String),
    /// Its model isn't installed (title).
    Model(String),
}

#[cfg_attr(not(feature = "sam"), allow(dead_code))]
fn model_title(key: &str) -> String {
    match key {
        "subject" => "BiRefNet-portrait (Subject)",
        "scene" => "EoMT (Sky, Landscape)",
        "people" => "MediaPipe Selfie Multiclass (People)",
        "face" => "MediaPipe Face Landmarker (faces)",
        "sam2" => "SAM 2.1 (Object, Describe, remembered objects)",
        "describe" => "OWLv2 object finder (Describe)",
        _ => key,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_of_selections() {
        let side = 10;
        let l: Vec<f32> = (0..side * side).map(|i| if (2..5).contains(&(i % side)) && (6..9).contains(&(i / side)) { 4.0 } else { -4.0 }).collect();
        let b = seg_box(&SegMask::from_logits(side, &l)).unwrap();
        assert!((b[0] - 0.2).abs() < 1e-6 && (b[1] - 0.6).abs() < 1e-6 && (b[2] - 0.5).abs() < 1e-6 && (b[3] - 0.9).abs() < 1e-6);
        assert!(seg_box(&SegMask::from_logits(side, &[-1.0; 100])).is_none());
        assert!(seg_box(&SegMask { side: 10, data: "?".into(), rect: None }).is_none());
    }
}
