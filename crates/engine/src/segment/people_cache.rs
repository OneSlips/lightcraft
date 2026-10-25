//! A photo's people, analysed once and kept (test notes 2026-10-25, note 10): "everyone's
//! clothes minus the bride's" on a set of photos computed every person's segmentation again for
//! the subtraction, photo after photo, though Everyone ▸ Clothes had just made it — the worker
//! keeps only the last photo's model outputs, and a batch goes through many photos.
//!
//! What a People selection needs of a photo, whoever and whichever part it is about, is kept per
//! photo as an [`Analysis`]: the faces (with landmarks), the Selfie Multiclass class map, a
//! group's per-person crops, everyone's SAM 2.1 silhouette and EoMT figure, the floors and walls
//! around them, and each face's appearance signature. In memory for the last [`MEMORY`] photos,
//! and on disk in the preview cache, so any later add, subtract or intersect of a person or a
//! part on that photo — after a restart too — runs only the mask itself.
//!
//! **Key** ([`key`]): the photo's model input (its file and look, as the worker's own caches are
//! keyed), [`VERSION`], this format's version and the models' file names and sizes (a model that
//! is replaced gives other results), as the eye records are.
//!
//! **Format** (a `.lcdata` record of [`lightcraft_preview::DiskCache`]): `LCPA`, the format
//! version (u32 LE), the length of a JSON header (u32 LE), the header (faces, signatures, which
//! parts there are and their sizes), then each part's values: probabilities as `u8` (1/255 steps),
//! logits as `i8` eighths clamped to ±16. A record that doesn't read is dropped and the photo
//! analysed again; it never stops anything.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use lightcraft_preview::{Hash128, Hasher128, PreviewCache};
use lightcraft_segment::identity::Signature;
use lightcraft_segment::mediapipe::Detection;
use lightcraft_segment::people::{Face, PeopleMap, Surfaces};
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 4] = b"LCPA";
/// The record's layout (bump when it changes; the key holds it too).
pub const FORMAT: u32 = 1;
/// What the analysis computes (bump when a step of it gives other results).
pub const VERSION: u64 = 1;
/// Photos kept in memory (a group photo's analysis is ~10 MB there).
pub const MEMORY: usize = 8;
/// The largest header read.
const MAX_HEADER: usize = 16 << 20;
/// Most faces, crops, silhouettes or figures read.
const MAX_ITEMS: usize = 400;
/// The largest side of a map read.
const MAX_SIDE: usize = 4096;
/// Classes of the class map.
const CLASSES: usize = 6;
/// Logits are kept to this range, in steps of 1/[`LOGIT_STEPS`].
const LOGIT_RANGE: f32 = 16.0;
const LOGIT_STEPS: f32 = 8.0;

/// A photo's people: whatever of it has been computed (`None`: not yet).
#[derive(Default)]
pub struct Analysis {
    /// The faces, left to right, with their landmarks (photo pixels of the model input).
    pub faces: Option<Vec<Face>>,
    /// Selfie Multiclass on the whole photo.
    pub map: Option<PeopleMap>,
    /// A group's small people segmented on their own crops: their boxes and maps (none: nobody
    /// small enough to need it).
    pub crowd: Option<(Vec<[f32; 4]>, Vec<PeopleMap>)>,
    /// Everyone's SAM 2.1 silhouette (one per face, `side²` logits; `Some(None)`: couldn't).
    pub silhouettes: Option<Option<Vec<Vec<f32>>>>,
    /// EoMT's people (`side²` logits each).
    pub figures: Option<Vec<Vec<f32>>>,
    /// The floors, walls and tables around them (`Some(None)`: EoMT couldn't run).
    pub surfaces: Option<Option<Surfaces>>,
    /// Each face's appearance signature, and whether it holds SAM 2.1's features.
    pub signatures: Option<(bool, Vec<Option<Signature>>)>,
}

impl Analysis {
    /// Whether `other` holds something this doesn't.
    pub fn adds(&self, other: &Analysis) -> bool {
        (self.faces.is_none() && other.faces.is_some())
            || (self.map.is_none() && other.map.is_some())
            || (self.crowd.is_none() && other.crowd.is_some())
            || (self.silhouettes.is_none() && other.silhouettes.is_some())
            || (self.figures.is_none() && other.figures.is_some())
            || (self.surfaces.is_none() && other.surfaces.is_some())
            || matches!((&self.signatures, &other.signatures), (None, Some(_)) | (Some((false, _)), Some((true, _))))
    }
}

/// Where a photo's analysis is kept on disk: the preview cache and the record's key.
#[derive(Clone)]
pub struct Store {
    pub cache: Arc<PreviewCache>,
    pub key: Hash128,
}

/// The names and sizes of the files in `dirs` (the models; `None`: not installed): a model that
/// is replaced (or one installed since) gives other results.
pub fn model_tag(dirs: &[Option<&Path>]) -> u64 {
    let mut h = Hasher128::new();
    for dir in dirs {
        let Some(dir) = dir else {
            h.str("-");
            continue;
        };
        let mut files: Vec<(String, u64)> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let m = e.metadata().ok().filter(|m| m.is_file())?;
                        Some((e.file_name().to_string_lossy().to_string(), m.len()))
                    })
                    .take(64)
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        h.str(&dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        for (name, len) in files {
            h.str(&name).u64(len);
        }
    }
    h.finish().0 as u64
}

/// The record's key for the model input `input_key` (the worker's key: the photo's file and look)
/// analysed with the models tagged `models` ([`model_tag`]).
pub fn key(input_key: u64, models: u64) -> Hash128 {
    Hasher128::new().str("people-analysis").u64(input_key).u64(VERSION).u64(u64::from(FORMAT)).u64(models).finish()
}

#[derive(Serialize, Deserialize)]
struct FaceRec {
    b: [f32; 4],
    s: f32,
    k: [[f32; 2]; 6],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    l: Option<Vec<[f32; 2]>>,
}

#[derive(Serialize, Deserialize, Default)]
struct Header {
    #[serde(default)]
    faces: Option<Vec<FaceRec>>,
    /// The class map's side.
    #[serde(default)]
    map: Option<usize>,
    /// The crops' boxes and their maps' sides.
    #[serde(default)]
    crowd: Option<(Vec<[f32; 4]>, Vec<usize>)>,
    /// Silhouettes: `(side, count)`.
    #[serde(default)]
    silhouettes: Option<(usize, usize)>,
    /// SAM 2.1 couldn't give them.
    #[serde(default)]
    no_silhouettes: bool,
    #[serde(default)]
    figures: Option<(usize, usize)>,
    #[serde(default)]
    surfaces: Option<(usize, usize)>,
    /// EoMT couldn't run.
    #[serde(default)]
    no_surfaces: bool,
    #[serde(default)]
    signatures: Option<(bool, Vec<Option<Signature>>)>,
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

fn probs_u8(p: &[f32]) -> impl Iterator<Item = u8> + '_ {
    p.iter().map(|v| (finite(*v).clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn logits_i8(l: &[f32]) -> impl Iterator<Item = u8> + '_ {
    l.iter().map(|v| ((finite(*v).clamp(-LOGIT_RANGE, LOGIT_RANGE) * LOGIT_STEPS).round() as i8) as u8)
}

/// The record of `a`; `None` when something in it is out of bounds.
pub fn encode(a: &Analysis) -> Option<Vec<u8>> {
    let mut h = Header {
        faces: a.faces.as_ref().map(|f| {
            f.iter()
                .map(|f| {
                    let d = &f.detection;
                    FaceRec { b: [d.x0, d.y0, d.x1, d.y1].map(finite), s: finite(d.score), k: d.keypoints, l: f.landmarks.clone() }
                })
                .collect()
        }),
        signatures: a.signatures.clone(),
        ..Header::default()
    };
    let mut blobs: Vec<u8> = Vec::new();
    if let Some(m) = &a.map {
        h.map = Some(m.side);
        blobs.extend(probs_u8(&m.probs));
    }
    if let Some((boxes, crops)) = &a.crowd {
        h.crowd = Some((boxes.clone(), crops.iter().map(|c| c.side).collect()));
        for c in crops {
            blobs.extend(probs_u8(&c.probs));
        }
    }
    let side_of = |v: &[Vec<f32>]| v.first().map_or(0, |l| (l.len() as f64).sqrt() as usize);
    match &a.silhouettes {
        Some(Some(s)) => {
            h.silhouettes = Some((side_of(s), s.len()));
            s.iter().for_each(|l| blobs.extend(logits_i8(l)));
        }
        Some(None) => h.no_silhouettes = true,
        None => {}
    }
    if let Some(f) = &a.figures {
        h.figures = Some((side_of(f), f.len()));
        f.iter().for_each(|l| blobs.extend(logits_i8(l)));
    }
    match &a.surfaces {
        Some(Some(s)) => {
            let (w, hh, p) = s.grid();
            h.surfaces = Some((w, hh));
            blobs.extend(probs_u8(p));
        }
        Some(None) => h.no_surfaces = true,
        None => {}
    }
    let json = serde_json::to_vec(&h).ok()?;
    let mut out = Vec::with_capacity(json.len() + 12 + blobs.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT.to_le_bytes());
    out.extend_from_slice(&u32::try_from(json.len()).ok()?.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&blobs);
    Some(out)
}

/// Reads a record from the front of a byte slice.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = rest;
        Some(head)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn probs(&mut self, n: usize) -> Option<Vec<f32>> {
        Some(self.take(n)?.iter().map(|v| f32::from(*v) / 255.0).collect())
    }
    fn logits(&mut self, n: usize) -> Option<Vec<f32>> {
        Some(self.take(n)?.iter().map(|v| f32::from(*v as i8) / LOGIT_STEPS).collect())
    }
}

fn map_len(side: usize) -> Option<usize> {
    (side > 0 && side <= MAX_SIDE).then(|| side * side * CLASSES)
}

fn square(side: usize, count: usize) -> Option<usize> {
    (side > 0 && side <= MAX_SIDE && count <= MAX_ITEMS).then(|| side * side)
}

/// The analysis in record `bytes` (`None`: not a record of this format, or damaged).
pub fn decode(bytes: &[u8]) -> Option<Analysis> {
    let mut r = Reader(bytes);
    if r.take(4)? != MAGIC || r.u32()? != FORMAT {
        return None;
    }
    let len = r.u32()? as usize;
    if len > MAX_HEADER {
        return None;
    }
    let h: Header = serde_json::from_slice(r.take(len)?).ok()?;
    let mut a = Analysis::default();
    if let Some(faces) = h.faces {
        if faces.len() > MAX_ITEMS || faces.iter().any(|f| f.l.as_ref().is_some_and(|l| l.len() > 1000)) {
            return None;
        }
        a.faces = Some(
            faces
                .into_iter()
                .map(|f| Face { detection: Detection { x0: f.b[0], y0: f.b[1], x1: f.b[2], y1: f.b[3], score: f.s, keypoints: f.k }, landmarks: f.l })
                .collect(),
        );
    }
    if let Some(side) = h.map {
        a.map = Some(PeopleMap { side, probs: r.probs(map_len(side)?)? });
    }
    if let Some((boxes, sides)) = h.crowd {
        if boxes.len() != sides.len() || boxes.len() > MAX_ITEMS {
            return None;
        }
        let mut crops = Vec::with_capacity(sides.len());
        for side in sides {
            crops.push(PeopleMap { side, probs: r.probs(map_len(side)?)? });
        }
        a.crowd = Some((boxes, crops));
    }
    if let Some((side, count)) = h.silhouettes {
        let n = square(side, count)?;
        a.silhouettes = Some(Some((0..count).map(|_| r.logits(n)).collect::<Option<Vec<_>>>()?));
    } else if h.no_silhouettes {
        a.silhouettes = Some(None);
    }
    if let Some((side, count)) = h.figures {
        let n = if count == 0 { 0 } else { square(side, count)? };
        a.figures = Some((0..count).map(|_| r.logits(n)).collect::<Option<Vec<_>>>()?);
    }
    if let Some((w, hh)) = h.surfaces {
        if w == 0 || hh == 0 || w > MAX_SIDE || hh > MAX_SIDE {
            return None;
        }
        a.surfaces = Some(Some(Surfaces::from_grid(w, hh, r.probs(w * hh)?)));
    } else if h.no_surfaces {
        a.surfaces = Some(None);
    }
    if let Some((features, sigs)) = h.signatures {
        if sigs.len() > MAX_ITEMS {
            return None;
        }
        a.signatures = Some((features, sigs));
    }
    if !r.0.is_empty() {
        return None;
    }
    Some(a)
}

/// The analysis kept in `store`; a record that doesn't read is dropped (`None`: analyse again).
pub fn read(store: &Store) -> Option<Analysis> {
    let bytes = store.cache.get_data(store.key)?;
    let a = decode(&bytes);
    if a.is_none() {
        log::warn!("people: dropping an unreadable analysis {} in the preview cache", store.key);
        store.cache.remove_data(store.key);
    }
    a
}

/// Keep `a` in `store`.
pub fn write(store: &Store, a: &Analysis) {
    if let Some(bytes) = encode(a) {
        store.cache.put_data(store.key, &bytes);
    }
}

/// The last [`MEMORY`] photos' analyses, by the worker's input key.
#[derive(Default)]
pub struct Memory {
    entries: VecDeque<(u64, Arc<Analysis>)>,
}

impl Memory {
    pub fn get(&mut self, key: u64) -> Option<Arc<Analysis>> {
        let i = self.entries.iter().position(|(k, _)| *k == key)?;
        let e = self.entries.remove(i)?;
        let a = e.1.clone();
        self.entries.push_front(e);
        Some(a)
    }

    pub fn put(&mut self, key: u64, a: Arc<Analysis>) {
        self.entries.retain(|(k, _)| *k != key);
        self.entries.push_front((key, a));
        self.entries.truncate(MEMORY);
    }
}

/// A copy of class map `m`.
pub fn copy_map(m: &PeopleMap) -> PeopleMap {
    PeopleMap { side: m.side, probs: m.probs.clone() }
}

/// A copy of `s`.
pub fn copy_surfaces(s: &Surfaces) -> Surfaces {
    let (w, h, p) = s.grid();
    Surfaces::from_grid(w, h, p.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(x: f32) -> Face {
        Face {
            detection: Detection { x0: x, y0: 10.0, x1: x + 20.0, y1: 34.0, score: 0.9, keypoints: [[x + 5.0, 18.0]; 6] },
            landmarks: Some(vec![[x + 1.0, 12.0]; 478]),
        }
    }

    fn sample() -> Analysis {
        let map = |side: usize, v: f32| PeopleMap { side, probs: (0..side * side * CLASSES).map(|i| (i % 7) as f32 / 6.0 * v).collect() };
        Analysis {
            faces: Some(vec![face(10.0), face(60.0)]),
            map: Some(map(16, 1.0)),
            crowd: Some((vec![[0.1, 0.1, 0.4, 0.9], [0.5, 0.1, 0.8, 0.9]], vec![map(8, 0.5), map(8, 1.0)])),
            silhouettes: Some(Some(vec![(0..64).map(|i| i as f32 / 2.0 - 16.0).collect(), vec![-20.0; 64]])),
            figures: Some(vec![(0..64).map(|i| (i as f32).sin() * 5.0).collect()]),
            surfaces: Some(Some(Surfaces::from_grid(4, 3, (0..12).map(|i| i as f32 / 11.0).collect()))),
            signatures: Some((true, vec![Some(Signature { hair: Some([0.1, 0.2, 0.3]), ..Default::default() }), None])),
        }
    }

    /// What goes in comes out (to the steps kept: 1/255, 1/8 of a logit, ±16).
    #[test]
    fn an_analysis_survives_its_record() {
        let a = sample();
        let b = decode(&encode(&a).unwrap()).unwrap();
        let (fa, fb) = (a.faces.as_ref().unwrap(), b.faces.as_ref().unwrap());
        assert_eq!(fa.len(), fb.len());
        assert_eq!(fa[1].detection, fb[1].detection);
        assert_eq!(fa[1].landmarks, fb[1].landmarks);
        let close = |x: &[f32], y: &[f32], tol: f32| x.len() == y.len() && x.iter().zip(y).all(|(p, q)| (p - q).abs() <= tol);
        assert!(close(&a.map.as_ref().unwrap().probs, &b.map.as_ref().unwrap().probs, 0.5 / 255.0 + 1e-6));
        let (ca, cb) = (a.crowd.as_ref().unwrap(), b.crowd.as_ref().unwrap());
        assert_eq!(ca.0, cb.0);
        assert!(ca.1.iter().zip(&cb.1).all(|(x, y)| x.side == y.side && close(&x.probs, &y.probs, 0.5 / 255.0 + 1e-6)));
        let (sa, sb) = (a.silhouettes.as_ref().unwrap().as_ref().unwrap(), b.silhouettes.as_ref().unwrap().as_ref().unwrap());
        assert!(close(&sa[0], &sb[0], 0.5 / LOGIT_STEPS + 1e-6));
        assert!(sb[1].iter().all(|v| *v == -LOGIT_RANGE), "clamped");
        assert!(close(&a.figures.as_ref().unwrap()[0], &b.figures.as_ref().unwrap()[0], 0.5 / LOGIT_STEPS + 1e-6));
        let (s1, s2) = (a.surfaces.as_ref().unwrap().as_ref().unwrap(), b.surfaces.as_ref().unwrap().as_ref().unwrap());
        assert!(close(s1.grid().2, s2.grid().2, 0.5 / 255.0 + 1e-6));
        assert_eq!(a.signatures, b.signatures);
        // nothing computed yet is nothing, and "couldn't" stays "couldn't"
        let empty = decode(&encode(&Analysis::default()).unwrap()).unwrap();
        assert!(empty.faces.is_none() && empty.map.is_none() && empty.silhouettes.is_none() && empty.signatures.is_none());
        let couldnt = Analysis { silhouettes: Some(None), surfaces: Some(None), ..Default::default() };
        let c = decode(&encode(&couldnt).unwrap()).unwrap();
        assert!(matches!(c.silhouettes, Some(None)) && matches!(c.surfaces, Some(None)));
        assert!(Analysis::default().adds(&a) && !a.adds(&Analysis::default()));
    }

    /// A damaged, cut short or foreign record is no analysis (and never a panic).
    #[test]
    fn a_bad_record_is_none() {
        let bytes = encode(&sample()).unwrap();
        for n in [0, 3, 8, 11, 12, 40, bytes.len() / 2, bytes.len() - 1] {
            assert!(decode(&bytes[..n]).is_none(), "cut at {n}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer).is_none(), "trailing bytes");
        let mut other = bytes.clone();
        other[4] = 99;
        assert!(decode(&other).is_none(), "another format");
        // a header claiming huge maps or counts
        for header in [
            r#"{"map": 100000}"#,
            r#"{"silhouettes": [64, 100000]}"#,
            r#"{"crowd": [[[0,0,1,1]], [8, 8]]}"#,
            r#"{"surfaces": [0, 5]}"#,
            r#"{"faces": "x"}"#,
        ] {
            let mut b = Vec::new();
            b.extend_from_slice(MAGIC);
            b.extend_from_slice(&FORMAT.to_le_bytes());
            b.extend_from_slice(&(header.len() as u32).to_le_bytes());
            b.extend_from_slice(header.as_bytes());
            assert!(decode(&b).is_none(), "{header}");
        }
    }

    #[test]
    fn memory_keeps_the_last_photos() {
        let mut m = Memory::default();
        for k in 0..(MEMORY as u64 + 3) {
            m.put(k, Arc::new(Analysis::default()));
        }
        assert!(m.get(0).is_none() && m.get(2).is_none(), "the oldest went");
        assert!(m.get(3).is_some(), "used: now the newest");
        m.put(100, Arc::new(Analysis::default()));
        assert!(m.get(3).is_some() && m.get(4).is_none());
    }
}
