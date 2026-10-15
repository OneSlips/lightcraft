//! The AI mask models LightCraft knows: what each one is for, the files it needs (pinned by size and
//! SHA-256), where it may be downloaded from, and its licence.
//!
//! None of them is part of LightCraft: each is downloaded once, into its own folder of the models
//! folder, only after the user agreed to it (the app names the model, its size and its licence
//! first). Everything works without them: AI masks then fall back to heuristics (Subject, Sky,
//! Background) or say that the model is needed (Object).
//!
//! The openly licensed models (SAM 2.1, OWLv2, EoMT, BiRefNet, Depth Anything V2 Small) are Apache-2.0 or MIT,
//! ungated, and may be redistributed: their default download location is the publisher's own
//! Hugging Face repository, pinned to a commit. SAM 3 is gated under Meta's SAM License and has no
//! default location (see `fetch`).
//!
//! Licences were checked on the official model cards and repositories on 2026-10-14 (the URLs are
//! in each entry and in docs/ai-masks.md).

use std::path::Path;

use lightcraft_fetch::FileSpec;

/// What a model selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelId {
    /// Object (clicks) and Describe (text), Meta's SAM 3 (gated, SAM License): optional, used
    /// when installed.
    Sam3,
    /// Describe (text): OWLv2 base (Apache-2.0) finds the boxes SAM 2.1 segments.
    Describe,
    /// Object (clicks): SAM 2.1 Hiera-Tiny (Apache-2.0).
    Sam2,
    /// Subject: BiRefNet-portrait (MIT), fine hair and edges.
    Subject,
    /// Sky, People, Background, Landscape: EoMT-S COCO panoptic (MIT).
    Scene,
    /// Depth: Depth Anything V2 Small (Apache-2.0). Not offered: its training data (SA-1B's
    /// derivative clause, Virtual KITTI 2 non-commercial) is a likely reject (docs/ai-masks.md).
    Depth,
    /// People: hair, face skin, body skin, clothes — MediaPipe Selfie Multiclass (Apache-2.0, Google).
    People,
    /// Faces: BlazeFace + FaceMesh-V2 landmarks for eyes, irises, brows, lips, teeth — MediaPipe
    /// Face Landmarker (Apache-2.0, Google).
    Face,
}

/// One downloadable model.
#[derive(Debug)]
pub struct ModelInfo {
    pub id: ModelId,
    /// Stable id for commands and settings (`ai.models.download {"id": …}`).
    pub key: &'static str,
    /// Folder inside the models folder.
    pub dir_name: &'static str,
    /// Shown to the user.
    pub title: &'static str,
    /// What it does, for the download dialog.
    pub purpose: &'static str,
    pub files: &'static [FileSpec<'static>],
    pub license_name: &'static str,
    pub license_url: &'static str,
    /// The publisher's page for the model.
    pub page_url: &'static str,
    /// Built-in download locations (base URLs: file `f` is at `<base>/<f>`), in order.
    pub default_mirrors: &'static [&'static str],
    /// Environment variable with extra mirrors, tried first.
    pub mirrors_env: &'static str,
    /// Environment variable with a bearer token for the mirror's host (gated repositories).
    pub token_env: Option<&'static str>,
    /// May LightCraft's own mirrors redistribute it (permissive licence, not gated)?
    pub open: bool,
    /// The licence of its training data is a maintainer decision still to make (tier C in
    /// docs/ai-masks.md): the app says so wherever it offers the model.
    pub licence_pending: bool,
    /// Extra files from the same publisher under the same licence, fetched from their own
    /// address with the model (and for an installed model that lacks them). The model works
    /// without them: they are not part of [`ModelInfo::installed_in`].
    pub companions: &'static [Companion],
}

/// An optional file that comes with a model (see [`ModelInfo::companions`]).
#[derive(Debug)]
pub struct Companion {
    pub file: FileSpec<'static>,
    /// Its exact download address.
    pub url: &'static str,
}

impl ModelInfo {
    /// The companions missing from `dir` (or there with the wrong size).
    pub fn missing_companions(&self, dir: &Path) -> Vec<&'static Companion> {
        self.companions
            .iter()
            .filter(|c| !matches!((std::fs::metadata(dir.join(c.file.name)), c.file.size), (Ok(m), Some(size)) if m.is_file() && m.len() == size))
            .collect()
    }
}

impl ModelInfo {
    /// Bytes to download (the pinned files and the companions).
    pub fn size(&self) -> u64 {
        self.files.iter().chain(self.companions.iter().map(|c| &c.file)).filter_map(|f| f.size).sum()
    }

    /// Whether `dir` holds the model: every file is there, pinned ones with their exact size.
    /// (The SHA-256 is checked when a file is downloaded, not on every launch.)
    pub fn installed_in(&self, dir: &Path) -> bool {
        self.files.iter().all(|f| {
            let p = dir.join(f.name);
            match (std::fs::metadata(&p), f.size) {
                (Ok(m), Some(size)) => m.is_file() && m.len() == size,
                (Ok(m), None) => m.is_file(),
                (Err(_), _) => false,
            }
        })
    }

    /// The mirrors to try, in order: the environment variable's, the mirrors file's (one base URL
    /// per line, `#` comments), then the built-in ones.
    pub fn mirrors(&self, env: Option<&str>, file: Option<&Path>) -> Vec<String> {
        lightcraft_fetch::mirrors(env, file, self.default_mirrors)
    }

    /// Download settings (a token only for a gated model's own host).
    pub fn options(&self) -> lightcraft_fetch::Options {
        lightcraft_fetch::Options { token_env: self.token_env, ..lightcraft_fetch::Options::default() }
    }

    /// The user's mirror list for this model, next to the model folders.
    pub fn mirrors_file_name(&self) -> String {
        format!("{}-mirrors.txt", self.dir_name)
    }
}

/// SAM 2.1 Hiera-Tiny, transformers format (`facebook/sam2.1-hiera-tiny`).
pub const SAM2_FILES: &[FileSpec<'static>] = &[
    FileSpec { name: "config.json", size: Some(5_695), sha256: Some("860aff9751b139d83a4ad7df1e5535416fded533e0ead02625edbefcb9953cce"), max: 5_695 },
    FileSpec {
        name: "model.safetensors",
        size: Some(155_908_064),
        sha256: Some("48c14467e5cf9e51870511feb72c89688e82dd74523142c0538b663e193ac2a7"),
        max: 155_908_064,
    },
];

/// OWLv2 base patch16 ensemble, transformers format (`google/owlv2-base-patch16-ensemble`).
pub const DESCRIBE_FILES: &[FileSpec<'static>] = &[
    FileSpec { name: "config.json", size: Some(414), sha256: Some("ba9df8c25a4b8461887dd0a93d9252c9cd84697fe8d49a9d8794ce409af9acb2"), max: 414 },
    FileSpec {
        name: "vocab.json",
        size: Some(1_059_962),
        sha256: Some("e089ad92ba36837a0d31433e555c8f45fe601ab5c221d4f607ded32d9f7a4349"),
        max: 1_059_962,
    },
    FileSpec {
        name: "merges.txt",
        size: Some(524_619),
        sha256: Some("9fd691f7c8039210e0fced15865466c65820d09b63988b0174bfe25de299051a"),
        max: 524_619,
    },
    FileSpec {
        name: "model.safetensors",
        size: Some(619_918_824),
        sha256: Some("e1e130b9e404cf91a75ad45644c1da9d7fa5284085eecc864266a6923efb99e7"),
        max: 619_918_824,
    },
];

/// BiRefNet-portrait (`ZhengPeng7/BiRefNet-portrait`, Swin-L, fp32).
pub const SUBJECT_FILES: &[FileSpec<'static>] = &[FileSpec {
    name: "model.safetensors",
    size: Some(884_878_856),
    sha256: Some("4a4eb3a5469b75f0cccaec6772c22fc30e6c12ed429c2c0ba71b43e3d8d97182"),
    max: 884_878_856,
}];

/// EoMT-S 2× COCO panoptic, 640 px (`tue-mps/coco_panoptic_eomt_small_640_2x`); `config.json`
/// is checked by the loader (the right architecture).
pub const SCENE_FILES: &[FileSpec<'static>] = &[
    FileSpec { name: "config.json", size: Some(6_426), sha256: Some("de222958bcd41d63e57059ffb42bfedbe6dddc6f8ac61529af8558e86667d2cc"), max: 6_426 },
    FileSpec {
        name: "model.safetensors",
        size: Some(95_927_796),
        sha256: Some("516b0b23acace639cc27739593881dda1c0010b3bec06aae138b615717fb8855"),
        max: 95_927_796,
    },
];

/// Depth Anything V2 Small, transformers format (`depth-anything/Depth-Anything-V2-Small-hf`).
pub const DEPTH_FILES: &[FileSpec<'static>] = &[
    FileSpec { name: "config.json", size: Some(950), sha256: Some("c56698d3643dde1f83ea2212759e6b31a22b8f827246a36dd007ee8a22b3ff75"), max: 950 },
    FileSpec {
        name: "model.safetensors",
        size: Some(99_173_660),
        sha256: Some("3152477ce0d8d6978d76b995120de97cb5b928701fd0f817769f59e249a16b70"),
        max: 99_173_660,
    },
];

// TODO(maintainer): put LightCraft's own mirror first in each openly licensed model's list (https, the same
// files unchanged, with the model's LICENSE next to them), e.g. "https://<cdn-host>/models/<dir_name>/<revision>".
// The publishers' repositories below stay as fallbacks; they are pinned to a commit, so the files
// can't change under the pinned hashes.

pub const SUBJECT: ModelInfo = ModelInfo {
    id: ModelId::Subject,
    key: "subject",
    dir_name: "birefnet-portrait",
    title: "BiRefNet portrait",
    purpose: "Select Subject: people with fine hair and edges",
    files: SUBJECT_FILES,
    license_name: "MIT",
    license_url: "https://github.com/ZhengPeng7/BiRefNet/blob/main/LICENSE",
    page_url: "https://huggingface.co/ZhengPeng7/BiRefNet-portrait",
    default_mirrors: &["https://huggingface.co/ZhengPeng7/BiRefNet-portrait/resolve/b6561965a70070d9143fd9e558f6ca3c481510db"],
    mirrors_env: "LIGHTCRAFT_SUBJECT_MIRRORS",
    token_env: None,
    open: true,
    licence_pending: true,
    companions: &[],
};

pub const SCENE: ModelInfo = ModelInfo {
    id: ModelId::Scene,
    key: "scene",
    dir_name: "eomt-s-coco",
    title: "EoMT scene",
    purpose: "Sky, People, Background and Landscape",
    files: SCENE_FILES,
    license_name: "MIT",
    license_url: "https://github.com/tue-mps/eomt/blob/master/LICENSE",
    page_url: "https://huggingface.co/tue-mps/coco_panoptic_eomt_small_640_2x",
    default_mirrors: &["https://huggingface.co/tue-mps/coco_panoptic_eomt_small_640_2x/resolve/10f5326794a8ce7e0e975791033ae1c13aec3767"],
    mirrors_env: "LIGHTCRAFT_SCENE_MIRRORS",
    token_env: None,
    open: true,
    licence_pending: true,
    companions: &[],
};

pub const SAM2: ModelInfo = ModelInfo {
    id: ModelId::Sam2,
    key: "sam2",
    dir_name: "sam2.1-hiera-tiny",
    title: "SAM 2.1",
    purpose: "Select Object: click what to select",
    files: SAM2_FILES,
    license_name: "Apache-2.0",
    license_url: "https://github.com/facebookresearch/sam2/blob/main/LICENSE",
    page_url: "https://huggingface.co/facebook/sam2.1-hiera-tiny",
    default_mirrors: &["https://huggingface.co/facebook/sam2.1-hiera-tiny/resolve/de431c4043854a71d8101e17995dfe596bf101a5"],
    mirrors_env: "LIGHTCRAFT_SAM2_MIRRORS",
    token_env: None,
    open: true,
    licence_pending: false,
    companions: &[],
};

pub const DESCRIBE: ModelInfo = ModelInfo {
    id: ModelId::Describe,
    key: "describe",
    dir_name: "owlv2-base-patch16-ensemble",
    title: "OWLv2 object finder",
    purpose: "Describe: select what you type (with SAM 2.1)",
    files: DESCRIBE_FILES,
    license_name: "Apache-2.0",
    license_url: "https://huggingface.co/google/owlv2-base-patch16-ensemble",
    page_url: "https://huggingface.co/google/owlv2-base-patch16-ensemble",
    default_mirrors: &["https://huggingface.co/google/owlv2-base-patch16-ensemble/resolve/cfd3195ba4ea9592eec887ded089f4c08eff231d"],
    mirrors_env: "LIGHTCRAFT_DESCRIBE_MIRRORS",
    token_env: None,
    open: true,
    // trained on WebLI pseudo-labels (web image-text pairs), docs/ai-masks.md
    licence_pending: true,
};

pub const DEPTH: ModelInfo = ModelInfo {
    id: ModelId::Depth,
    key: "depth",
    dir_name: "depth-anything-v2-small",
    title: "Depth Anything V2 Small",
    purpose: "Depth Range masks",
    files: DEPTH_FILES,
    license_name: "Apache-2.0",
    license_url: "https://huggingface.co/depth-anything/Depth-Anything-V2-Small-hf",
    page_url: "https://huggingface.co/depth-anything/Depth-Anything-V2-Small-hf",
    default_mirrors: &["https://huggingface.co/depth-anything/Depth-Anything-V2-Small-hf/resolve/5426e4f0f36572d16453bbda7a8389317b1bef99"],
    mirrors_env: "LIGHTCRAFT_DEPTH_MIRRORS",
    token_env: None,
    open: true,
    licence_pending: false,
    companions: &[],
};

pub const SAM3: ModelInfo = ModelInfo {
    id: ModelId::Sam3,
    key: "sam3",
    dir_name: "sam3",
    title: "SAM 3",
    purpose: "Optional: Select Object and Describe with Meta's model (gated licence); both work without it",
    files: crate::fetch::SAM3_FILES,
    license_name: "SAM License (Meta)",
    license_url: "https://github.com/facebookresearch/sam3/blob/main/LICENSE",
    page_url: "https://huggingface.co/facebook/sam3",
    default_mirrors: crate::fetch::DEFAULT_MIRRORS,
    mirrors_env: crate::fetch::MIRRORS_ENV,
    token_env: Some(crate::fetch::TOKEN_ENV),
    open: false,
    licence_pending: false,
    companions: &[],
};

/// MediaPipe Selfie Multiclass 256 (float32, version 1).
pub const PEOPLE_FILES: &[FileSpec<'static>] = &[FileSpec {
    name: "selfie_multiclass_256x256.tflite",
    size: Some(16_371_837),
    sha256: Some("c6748b1253a99067ef71f7e26ca71096cd449baefa8f101900ea23016507e0e0"),
    max: 16_371_837,
}];

/// MediaPipe Face Landmarker bundle (float16, version 1): a zip of BlazeFace (short range),
/// FaceMesh-V2 and the blendshapes model (unused).
pub const FACE_FILES: &[FileSpec<'static>] = &[FileSpec {
    name: "face_landmarker.task",
    size: Some(3_758_596),
    sha256: Some("64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff"),
    max: 3_758_596,
}];

pub const PEOPLE: ModelInfo = ModelInfo {
    id: ModelId::People,
    key: "people",
    dir_name: "mediapipe-selfie-multiclass",
    title: "MediaPipe people",
    purpose: "People: hair, face skin, body skin and clothes",
    files: PEOPLE_FILES,
    license_name: "Apache-2.0",
    license_url: "https://storage.googleapis.com/mediapipe-assets/Model%20Card%20Multiclass%20Segmentation.pdf",
    page_url: "https://ai.google.dev/edge/mediapipe/solutions/vision/image_segmenter",
    default_mirrors: &["https://storage.googleapis.com/mediapipe-models/image_segmenter/selfie_multiclass_256x256/float32/1"],
    mirrors_env: "LIGHTCRAFT_PEOPLE_MIRRORS",
    token_env: None,
    open: true,
    licence_pending: false,
    companions: &[],
};

/// MediaPipe BlazeFace full range (float32, Apache-2.0, Google): finds the small faces of group
/// photos that the bundle's short-range detector misses.
pub const FACE_FAR: Companion = Companion {
    file: FileSpec {
        name: "face_detection_full_range.tflite",
        size: Some(1_083_786),
        sha256: Some("3698b18f063835bc609069ef052228fbe86d9c9a6dc8dcb7c7c2d69aed2b181b"),
        max: 1_083_786,
    },
    url: "https://storage.googleapis.com/mediapipe-assets/face_detection_full_range.tflite",
};

pub const FACE: ModelInfo = ModelInfo {
    id: ModelId::Face,
    key: "face",
    dir_name: "mediapipe-face-landmarker",
    title: "MediaPipe faces",
    purpose: "People: eyes, irises, eyebrows, lips and teeth",
    files: FACE_FILES,
    license_name: "Apache-2.0",
    license_url: "https://storage.googleapis.com/mediapipe-assets/Model%20Card%20MediaPipe%20Face%20Mesh%20V2.pdf",
    page_url: "https://ai.google.dev/edge/mediapipe/solutions/vision/face_landmarker",
    default_mirrors: &["https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1"],
    mirrors_env: "LIGHTCRAFT_FACE_MIRRORS",
    token_env: None,
    open: true,
    licence_pending: false,
    companions: &[FACE_FAR],
};

/// Every model the app offers, in the order it lists them (Depth Anything is ported but not
/// offered, see [`ModelId::Depth`]).
pub const MODELS: &[ModelInfo] = &[PEOPLE, FACE, SUBJECT, SCENE, SAM2, DESCRIBE, SAM3];

/// The entry for `id`.
pub fn info(id: ModelId) -> &'static ModelInfo {
    match id {
        ModelId::Sam3 => &SAM3,
        ModelId::Sam2 => &SAM2,
        ModelId::Describe => &DESCRIBE,
        ModelId::Subject => &SUBJECT,
        ModelId::Scene => &SCENE,
        ModelId::Depth => &DEPTH,
        ModelId::People => &PEOPLE,
        ModelId::Face => &FACE,
    }
}

/// The entry with command key `key` (`"sam2"`, `"subject"`, …).
pub fn by_key(key: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_has_one_entry_with_pinned_files() {
        for id in [ModelId::Sam3, ModelId::Sam2, ModelId::Describe, ModelId::Subject, ModelId::Scene, ModelId::People, ModelId::Face] {
            assert_eq!(MODELS.iter().filter(|m| m.id == id).count(), 1, "{id:?}");
            assert_eq!(info(id).id, id);
        }
        let mut keys: Vec<_> = MODELS.iter().map(|m| m.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), MODELS.len(), "keys are unique");
        for m in MODELS.iter().filter(|m| m.open) {
            assert!(!m.default_mirrors.is_empty(), "{}: an openly licensed model downloads without configuration", m.key);
            assert!(m.default_mirrors.iter().all(|u| u.starts_with("https://")), "{}", m.key);
            assert!(m.files.iter().all(|f| f.size.is_some() && f.sha256.is_some_and(|h| h.len() == 64)), "{}: pinned", m.key);
            assert!(m.token_env.is_none(), "{}: openly licensed models never send a token", m.key);
        }
        assert_eq!(by_key("scene").map(|m| m.id), Some(ModelId::Scene));
        // Describe's open detector: a tier C model (training data licence pending)
        let d = by_key("describe").unwrap();
        assert!(d.open && d.licence_pending && d.token_env.is_none());
        assert!(crate::owl::OWL_FILES.iter().all(|f| d.files.iter().any(|s| s.name == *f)), "the loader's files are the pinned ones");
        assert!(by_key("nope").is_none());
    }

    #[test]
    fn installed_means_every_file_with_its_pinned_size() {
        let dir = std::env::temp_dir().join(format!("lc-models-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let m = info(ModelId::Scene);
        assert!(!m.installed_in(&dir));
        // a truncated file is not an installed model
        std::fs::write(dir.join("model.safetensors"), b"short").unwrap();
        assert!(!m.installed_in(&dir));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!m.installed_in(Path::new("/nonexistent/lc")));
    }

    #[test]
    fn mirrors_put_the_users_first() {
        let m = info(ModelId::Sam2);
        let list = m.mirrors(Some("https://mine.example/m"), None);
        assert_eq!(list.first().map(String::as_str), Some("https://mine.example/m"));
        assert_eq!(list.len(), 1 + m.default_mirrors.len());
        assert_eq!(m.mirrors_file_name(), "sam2.1-hiera-tiny-mirrors.txt");
    }
}
