//! Persisted Playlist configuration and built-in image source selection.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const PLAYLIST_FORMAT_VERSION: u32 = 1;
pub const DEFAULT_INTERVAL_MINUTES: u16 = 15;
pub const MIN_INTERVAL_MINUTES: u16 = 1;
pub const MAX_INTERVAL_MINUTES: u16 = 1_440;
pub const DEFAULT_PLAYLIST_TEMPLATE_NAME: &str = "default-playlist.json";
pub const DEFAULT_PLAYLIST_IMAGE_FOLDER: &str = "images/default-playlist";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ItemId(u64);

impl ItemId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlaylistRevision {
    pub format_version: u32,
    pub revision: u64,
    pub default_interval_minutes: u16,
    pub next_item_id: u64,
    pub items: Vec<PlaylistItem>,
}

impl Default for PlaylistRevision {
    fn default() -> Self {
        Self {
            format_version: PLAYLIST_FORMAT_VERSION,
            revision: 0,
            default_interval_minutes: DEFAULT_INTERVAL_MINUTES,
            next_item_id: 1,
            items: Vec::new(),
        }
    }
}

impl PlaylistRevision {
    pub fn add_item(&mut self, title: String, source: PlaylistSource) -> ItemId {
        let id = ItemId(self.next_item_id);
        self.next_item_id = self.next_item_id.saturating_add(1);
        self.items.push(PlaylistItem {
            id,
            title,
            enabled: true,
            interval_minutes: None,
            source,
        });
        id
    }

    /// Move one stable Playlist Item identity relative to another.
    ///
    /// Returns `true` only when the order changed. Revision numbers remain the
    /// responsibility of [`PlaylistStore::save_new_revision`].
    pub fn move_item_to(
        &mut self,
        dragged_id: ItemId,
        target_id: ItemId,
        after_target: bool,
    ) -> bool {
        let Some(source_index) = self.items.iter().position(|item| item.id == dragged_id) else {
            return false;
        };
        let Some(target_index) = self.items.iter().position(|item| item.id == target_id) else {
            return false;
        };
        if source_index == target_index {
            return false;
        }

        let mut destination = target_index + usize::from(after_target);
        if source_index < destination {
            destination -= 1;
        }
        if source_index == destination {
            return false;
        }

        let item = self.items.remove(source_index);
        self.items.insert(destination, item);
        true
    }

    #[must_use]
    pub fn effective_interval_minutes(&self, item: &PlaylistItem) -> u16 {
        item.interval_minutes
            .unwrap_or(self.default_interval_minutes)
    }

    pub fn validate(&self) -> Result<(), PlaylistError> {
        if self.format_version != PLAYLIST_FORMAT_VERSION {
            return Err(PlaylistError::UnsupportedVersion(self.format_version));
        }
        validate_interval(self.default_interval_minutes, "playlist default")?;
        let mut ids = BTreeSet::new();
        for item in &self.items {
            if !ids.insert(item.id) {
                return Err(PlaylistError::DuplicateItemId(item.id));
            }
            if item.title.trim().is_empty() {
                return Err(PlaylistError::EmptyTitle(item.id));
            }
            if let Some(minutes) = item.interval_minutes {
                validate_interval(minutes, "item override")?;
            }
            item.source.validate()?;
        }
        if self
            .items
            .iter()
            .map(|item| item.id.get())
            .max()
            .is_some_and(|largest| self.next_item_id <= largest)
        {
            return Err(PlaylistError::InvalidNextItemId);
        }
        Ok(())
    }

    #[must_use]
    pub fn enabled_ids(&self) -> Vec<ItemId> {
        self.items
            .iter()
            .filter(|item| item.enabled)
            .map(|item| item.id)
            .collect()
    }

    #[must_use]
    pub fn item(&self, id: ItemId) -> Option<&PlaylistItem> {
        self.items.iter().find(|item| item.id == id)
    }

    #[must_use]
    pub fn next_enabled_after(&self, current: Option<ItemId>) -> Option<ItemId> {
        let enabled = self.enabled_ids();
        if enabled.is_empty() {
            return None;
        }
        let Some(current) = current else {
            return enabled.first().copied();
        };
        enabled.iter().position(|id| *id == current).map_or_else(
            || {
                let old_position = self
                    .items
                    .iter()
                    .position(|item| item.id == current)
                    .unwrap_or(0);
                self.items
                    .iter()
                    .skip(old_position.min(self.items.len()))
                    .chain(self.items.iter())
                    .find(|item| item.enabled)
                    .map(|item| item.id)
            },
            |position| enabled.get((position + 1) % enabled.len()).copied(),
        )
    }
}

fn validate_interval(minutes: u16, field: &'static str) -> Result<(), PlaylistError> {
    if (MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&minutes) {
        Ok(())
    } else {
        Err(PlaylistError::InvalidInterval { field, minutes })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlaylistItem {
    pub id: ItemId,
    pub title: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_minutes: Option<u16>,
    pub source: PlaylistSource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlaylistSource {
    Image {
        path: PathBuf,
    },
    ImageFolder {
        path: PathBuf,
    },
    Extension {
        path: PathBuf,
        #[serde(default)]
        settings: BTreeMap<String, Value>,
    },
}

impl PlaylistSource {
    fn validate(&self) -> Result<(), PlaylistError> {
        let path = match self {
            Self::Image { path } | Self::ImageFolder { path } | Self::Extension { path, .. } => {
                path
            }
        };
        if path.as_os_str().is_empty() {
            Err(PlaylistError::EmptySourcePath)
        } else {
            Ok(())
        }
    }

    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Image { .. } => "Image",
            Self::ImageFolder { .. } => "Image folder",
            Self::Extension { .. } => "Extension",
        }
    }

    #[must_use]
    pub fn display_path(&self) -> &Path {
        match self {
            Self::Image { path } | Self::ImageFolder { path } | Self::Extension { path, .. } => {
                path
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum PlaylistError {
    #[error("playlist format version {0} is not supported")]
    UnsupportedVersion(u32),
    #[error("{field} interval must be 1–1440 minutes, received {minutes}")]
    InvalidInterval { field: &'static str, minutes: u16 },
    #[error("playlist contains duplicate item id {0:?}")]
    DuplicateItemId(ItemId),
    #[error("playlist item {0:?} has an empty title")]
    EmptyTitle(ItemId),
    #[error("playlist next item id must be greater than every existing item id")]
    InvalidNextItemId,
    #[error("playlist source path is empty")]
    EmptySourcePath,
    #[error("could not access playlist revisions at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not decode playlist revision {path}: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not encode playlist revision: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("could not locate the WireTerm executable")]
    ExecutablePath,
    #[error("default Playlist source paths must be relative to adjacent WireTerm data: {0}")]
    NonPortableDefaultSource(PathBuf),
    #[error("default Playlist source is unavailable: {0}")]
    DefaultSource(#[source] SourceError),
}

#[derive(Clone, Debug)]
pub struct PlaylistStore {
    data_dir: PathBuf,
    revisions_dir: PathBuf,
}

impl PlaylistStore {
    pub fn adjacent_to_executable() -> Result<Self, PlaylistError> {
        let executable = std::env::current_exe().map_err(|_| PlaylistError::ExecutablePath)?;
        let parent = executable.parent().ok_or(PlaylistError::ExecutablePath)?;
        Ok(Self::new(&parent.join("wireterm-data")))
    }

    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            revisions_dir: data_dir.join("playlist-revisions"),
        }
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub fn revisions_dir(&self) -> &Path {
        &self.revisions_dir
    }

    #[must_use]
    pub fn resolve_source_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.data_dir.join(path)
        }
    }

    /// Load an existing immutable revision or create revision 1 from the
    /// adjacent portable template when no revision has ever been saved.
    pub fn load_or_initialize_default(&self) -> Result<PlaylistRevision, PlaylistError> {
        if self.has_revision_files()? {
            return self.load_latest();
        }
        let template_path = self.data_dir.join(DEFAULT_PLAYLIST_TEMPLATE_NAME);
        let bytes = match fs::read(&template_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(PlaylistRevision::default());
            }
            Err(source) => {
                return Err(PlaylistError::Io {
                    path: template_path,
                    source,
                });
            }
        };
        let template = serde_json::from_slice::<PlaylistRevision>(&bytes).map_err(|source| {
            PlaylistError::Decode {
                path: template_path,
                source,
            }
        })?;
        template.validate()?;
        for item in &template.items {
            let path = item.source.display_path();
            if !is_safe_relative_asset_path(path) {
                return Err(PlaylistError::NonPortableDefaultSource(path.to_path_buf()));
            }
            if let PlaylistSource::ImageFolder { path } = &item.source {
                let resolved = self.resolve_source_path(path);
                if scan_image_folder(&resolved)
                    .map_err(PlaylistError::DefaultSource)?
                    .is_empty()
                {
                    return Err(PlaylistError::DefaultSource(SourceError::EmptyFolder(
                        resolved,
                    )));
                }
            }
        }
        self.save_new_revision(template)
    }

    fn has_revision_files(&self) -> Result<bool, PlaylistError> {
        let entries = match fs::read_dir(&self.revisions_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(source) => {
                return Err(PlaylistError::Io {
                    path: self.revisions_dir.clone(),
                    source,
                });
            }
        };
        Ok(entries
            .filter_map(Result::ok)
            .any(|entry| is_revision_path(&entry.path())))
    }

    pub fn load_latest(&self) -> Result<PlaylistRevision, PlaylistError> {
        let entries = match fs::read_dir(&self.revisions_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(PlaylistRevision::default());
            }
            Err(source) => {
                return Err(PlaylistError::Io {
                    path: self.revisions_dir.clone(),
                    source,
                });
            }
        };
        let mut candidates = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_revision_path(path))
            .collect::<Vec<_>>();
        candidates.sort_unstable_by(|left, right| right.file_name().cmp(&left.file_name()));

        let mut last_error = None;
        for path in candidates {
            match fs::read(&path) {
                Ok(bytes) => match serde_json::from_slice::<PlaylistRevision>(&bytes) {
                    Ok(revision) => match revision.validate() {
                        Ok(()) => return Ok(revision),
                        Err(error) => last_error = Some(error),
                    },
                    Err(source) => {
                        last_error = Some(PlaylistError::Decode {
                            path: path.clone(),
                            source,
                        });
                    }
                },
                Err(source) => {
                    last_error = Some(PlaylistError::Io {
                        path: path.clone(),
                        source,
                    });
                }
            }
        }
        last_error.map_or_else(|| Ok(PlaylistRevision::default()), Err)
    }

    pub fn save_new_revision(
        &self,
        mut revision: PlaylistRevision,
    ) -> Result<PlaylistRevision, PlaylistError> {
        revision.validate()?;
        fs::create_dir_all(&self.revisions_dir).map_err(|source| PlaylistError::Io {
            path: self.revisions_dir.clone(),
            source,
        })?;

        let latest = self.load_latest()?;
        revision.revision = latest.revision.max(revision.revision).saturating_add(1);
        let bytes = serde_json::to_vec_pretty(&revision)?;
        let final_path = self
            .revisions_dir
            .join(format!("revision-{:020}.json", revision.revision));
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let temporary_path = self.revisions_dir.join(format!(
            ".revision-{:020}-{}-{nonce}.tmp",
            revision.revision,
            std::process::id()
        ));
        let write_result = write_revision_file(&temporary_path, &bytes).and_then(|()| {
            fs::rename(&temporary_path, &final_path).map_err(|source| PlaylistError::Io {
                path: final_path.clone(),
                source,
            })
        });
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result?;
        Ok(revision)
    }
}

fn is_revision_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with("revision-")
                && Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
}

fn write_revision_file(path: &Path, bytes: &[u8]) -> Result<(), PlaylistError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| PlaylistError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(bytes).map_err(|source| PlaylistError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.sync_all().map_err(|source| PlaylistError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("could not scan image folder {path}: {source}")]
    Scan {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("image folder {0} has no direct PNG or JPEG files")]
    EmptyFolder(PathBuf),
    #[error("preview is only available for built-in image items")]
    NotBuiltInImage,
}

#[derive(Default)]
struct ShuffleBag {
    eligible: BTreeSet<PathBuf>,
    remaining: Vec<PathBuf>,
}

/// Session-only shuffle bags. Constructing a new resolver resets all bags.
pub struct FolderShuffleBags {
    bags: HashMap<ItemId, ShuffleBag>,
    random_state: u64,
}

impl Default for FolderShuffleBags {
    fn default() -> Self {
        let seed =
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0xD1B5_4A32_D192_ED03, |time| {
                    let nanos = time.as_nanos();
                    u64::try_from(nanos).unwrap_or_else(|_| u64::try_from(nanos >> 64).unwrap_or(0))
                        ^ u64::from(std::process::id())
                });
        Self::with_seed(seed)
    }
}

impl FolderShuffleBags {
    #[must_use]
    pub fn with_seed(seed: u64) -> Self {
        Self {
            bags: HashMap::new(),
            random_state: seed.max(1),
        }
    }

    pub fn resolve_turn(&mut self, item_id: ItemId, folder: &Path) -> Result<PathBuf, SourceError> {
        let eligible = scan_image_folder(folder)?;
        if eligible.is_empty() {
            return Err(SourceError::EmptyFolder(folder.to_path_buf()));
        }
        let eligible_set = eligible.iter().cloned().collect::<BTreeSet<_>>();
        let bag = self.bags.entry(item_id).or_default();
        bag.remaining.retain(|path| eligible_set.contains(path));
        for path in eligible_set.difference(&bag.eligible) {
            bag.remaining.push(path.clone());
        }
        bag.eligible = eligible_set;
        if bag.remaining.is_empty() {
            bag.remaining.extend(eligible);
        }
        shuffle(&mut bag.remaining, &mut self.random_state);
        bag.remaining
            .pop()
            .ok_or_else(|| SourceError::EmptyFolder(folder.to_path_buf()))
    }

    pub fn preview(source: &PlaylistSource) -> Result<PathBuf, SourceError> {
        match source {
            PlaylistSource::Image { path } => Ok(path.clone()),
            PlaylistSource::ImageFolder { path } => scan_image_folder(path)?
                .into_iter()
                .next()
                .ok_or_else(|| SourceError::EmptyFolder(path.clone())),
            PlaylistSource::Extension { .. } => Err(SourceError::NotBuiltInImage),
        }
    }
}

fn shuffle(paths: &mut [PathBuf], state: &mut u64) {
    for index in (1..paths.len()).rev() {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        let bound = u64::try_from(index + 1).unwrap_or(u64::MAX);
        let target = usize::try_from(*state % bound).unwrap_or(0);
        paths.swap(index, target);
    }
}

pub fn scan_image_folder(folder: &Path) -> Result<Vec<PathBuf>, SourceError> {
    let entries = fs::read_dir(folder).map_err(|source| SourceError::Scan {
        path: folder.to_path_buf(),
        source,
    })?;
    let mut images = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_png_or_jpeg(&path) {
            images.push(path);
        }
    }
    images.sort_unstable();
    Ok(images)
}

#[must_use]
pub fn is_png_or_jpeg(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg"
            )
        })
}

#[must_use]
pub fn is_safe_relative_asset_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::time::Duration;

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("wireterm-{label}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn playlist_intervals_inherit_and_validate() {
        let mut playlist = PlaylistRevision::default();
        let id = playlist.add_item(
            "Photo".to_owned(),
            PlaylistSource::Image {
                path: "photo.png".into(),
            },
        );
        let item = playlist.item(id).expect("new item");
        assert_eq!(playlist.effective_interval_minutes(item), 15);

        playlist.items[0].interval_minutes = Some(42);
        assert_eq!(playlist.effective_interval_minutes(&playlist.items[0]), 42);
        playlist.items[0].interval_minutes = Some(0);
        assert!(matches!(
            playlist.validate(),
            Err(PlaylistError::InvalidInterval { .. })
        ));
    }

    #[test]
    fn revisions_are_immutable_and_latest_valid_wins() {
        let directory = TestDirectory::new("revisions");
        let store = PlaylistStore::new(&directory.0);
        let mut first = PlaylistRevision::default();
        first.add_item(
            "First".to_owned(),
            PlaylistSource::Image {
                path: "first.png".into(),
            },
        );
        let first = store.save_new_revision(first).expect("first save");
        let mut second = first.clone();
        second.items[0].title = "Second".to_owned();
        let second = store.save_new_revision(second).expect("second save");

        assert_eq!(first.revision, 1);
        assert_eq!(second.revision, 2);
        assert_eq!(
            store.load_latest().expect("latest").items[0].title,
            "Second"
        );
        assert_eq!(
            fs::read_dir(store.revisions_dir())
                .expect("revisions")
                .count(),
            2
        );
    }

    fn write_default_template(data_dir: &Path, source_path: &str) {
        let mut template = PlaylistRevision::default();
        template.add_item(
            "Red image collection".to_owned(),
            PlaylistSource::ImageFolder {
                path: source_path.into(),
            },
        );
        fs::write(
            data_dir.join(DEFAULT_PLAYLIST_TEMPLATE_NAME),
            serde_json::to_vec_pretty(&template).expect("encode template"),
        )
        .expect("write template");
    }

    #[test]
    fn fresh_data_initializes_portable_default_once() {
        let directory = TestDirectory::new("default-init");
        let image_folder = directory.0.join(DEFAULT_PLAYLIST_IMAGE_FOLDER);
        fs::create_dir_all(&image_folder).expect("image folder");
        File::create(image_folder.join("red.jpg")).expect("image fixture");
        write_default_template(&directory.0, DEFAULT_PLAYLIST_IMAGE_FOLDER);
        let store = PlaylistStore::new(&directory.0);

        let initialized = store
            .load_or_initialize_default()
            .expect("initialize default");
        let initialized_again = store
            .load_or_initialize_default()
            .expect("load initialized default");

        assert_eq!(initialized.revision, 1);
        assert_eq!(initialized_again, initialized);
        assert_eq!(initialized.default_interval_minutes, 15);
        assert_eq!(initialized.items.len(), 1);
        assert!(initialized.items[0].enabled);
        assert_eq!(initialized.items[0].interval_minutes, None);
        assert_eq!(
            initialized.items[0].source,
            PlaylistSource::ImageFolder {
                path: PathBuf::from(DEFAULT_PLAYLIST_IMAGE_FOLDER)
            }
        );
        assert_eq!(
            store.resolve_source_path(initialized.items[0].source.display_path()),
            image_folder
        );
        assert_eq!(
            fs::read_dir(store.revisions_dir())
                .expect("revision directory")
                .count(),
            1
        );
    }

    #[test]
    fn existing_revision_is_preserved_when_default_template_changes() {
        let directory = TestDirectory::new("default-preserve");
        let store = PlaylistStore::new(&directory.0);
        let mut existing = PlaylistRevision::default();
        let existing_id = existing.add_item(
            "Existing user item".to_owned(),
            PlaylistSource::Image {
                path: PathBuf::from("C:\\user\\chosen.png"),
            },
        );
        let existing = store
            .save_new_revision(existing)
            .expect("existing revision");
        write_default_template(&directory.0, "images/missing-default");

        let loaded = store
            .load_or_initialize_default()
            .expect("load existing revision");

        assert_eq!(loaded, existing);
        assert_eq!(
            loaded.item(existing_id).expect("existing item").title,
            "Existing user item"
        );
        assert_eq!(
            fs::read_dir(store.revisions_dir())
                .expect("revision directory")
                .count(),
            1
        );
    }

    #[test]
    fn default_template_rejects_nonportable_or_ineligible_folders() {
        let nonportable = TestDirectory::new("default-nonportable");
        write_default_template(&nonportable.0, "../outside");
        assert!(matches!(
            PlaylistStore::new(&nonportable.0).load_or_initialize_default(),
            Err(PlaylistError::NonPortableDefaultSource(_))
        ));
        assert!(!PlaylistStore::new(&nonportable.0).revisions_dir().exists());

        let empty = TestDirectory::new("default-empty");
        fs::create_dir_all(empty.0.join(DEFAULT_PLAYLIST_IMAGE_FOLDER).join("nested"))
            .expect("nested folder");
        File::create(
            empty
                .0
                .join(DEFAULT_PLAYLIST_IMAGE_FOLDER)
                .join("nested")
                .join("not-direct.jpg"),
        )
        .expect("nested fixture");
        write_default_template(&empty.0, DEFAULT_PLAYLIST_IMAGE_FOLDER);
        assert!(matches!(
            PlaylistStore::new(&empty.0).load_or_initialize_default(),
            Err(PlaylistError::DefaultSource(SourceError::EmptyFolder(_)))
        ));
        assert!(!PlaylistStore::new(&empty.0).revisions_dir().exists());
    }

    #[test]
    fn bundled_default_folder_is_exact_and_fully_decodable() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/default-playlist");
        let images = scan_image_folder(&folder).expect("scan bundled images");
        assert_eq!(images.len(), 16);
        assert_eq!(
            fs::read_dir(&folder).expect("read bundled folder").count(),
            images.len(),
            "the non-recursive folder must contain playlist JPEGs only"
        );
        for path in images {
            assert_eq!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("jpg")
            );
            assert_eq!(
                image::image_dimensions(&path).expect("decode image"),
                (800, 480)
            );
        }
    }

    #[test]
    fn reorder_moves_stable_item_identities_up_and_down() {
        let mut playlist = PlaylistRevision::default();
        let first = playlist.add_item(
            "First".to_owned(),
            PlaylistSource::Image {
                path: "first.png".into(),
            },
        );
        let second = playlist.add_item(
            "Second".to_owned(),
            PlaylistSource::Image {
                path: "second.png".into(),
            },
        );
        let third = playlist.add_item(
            "Third".to_owned(),
            PlaylistSource::Image {
                path: "third.png".into(),
            },
        );

        assert!(playlist.move_item_to(first, third, true));
        assert_eq!(
            playlist
                .items
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [second, third, first]
        );
        assert!(playlist.move_item_to(first, second, false));
        assert_eq!(
            playlist
                .items
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [first, second, third]
        );
        assert!(!playlist.move_item_to(second, third, false));
        assert_eq!(
            playlist
                .items
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [first, second, third]
        );
    }

    #[test]
    fn reordered_revision_persists_atomically() {
        let directory = TestDirectory::new("reorder-revision");
        let store = PlaylistStore::new(&directory.0);
        let mut playlist = PlaylistRevision::default();
        let first = playlist.add_item(
            "First".to_owned(),
            PlaylistSource::Image {
                path: "first.png".into(),
            },
        );
        let second = playlist.add_item(
            "Second".to_owned(),
            PlaylistSource::Image {
                path: "second.png".into(),
            },
        );
        let saved = store.save_new_revision(playlist).expect("initial revision");

        let mut reordered = saved.clone();
        assert!(reordered.move_item_to(second, first, false));
        let reordered = store
            .save_new_revision(reordered)
            .expect("reordered revision");
        let loaded = store.load_latest().expect("latest revision");

        assert_eq!(saved.revision, 1);
        assert_eq!(reordered.revision, 2);
        assert_eq!(loaded.revision, 2);
        assert_eq!(
            loaded.items.iter().map(|item| item.id).collect::<Vec<_>>(),
            [second, first]
        );
        assert_eq!(
            saved.items.iter().map(|item| item.id).collect::<Vec<_>>(),
            [first, second]
        );
    }

    #[test]
    fn folder_bag_rescans_without_preview_consumption() {
        let directory = TestDirectory::new("bag");
        for name in ["a.png", "b.jpg", "ignored.gif"] {
            File::create(directory.0.join(name)).expect("fixture");
        }
        fs::create_dir(directory.0.join("nested")).expect("nested directory");
        File::create(directory.0.join("nested").join("c.png")).expect("nested fixture");
        let source = PlaylistSource::ImageFolder {
            path: directory.0.clone(),
        };
        let first_preview = FolderShuffleBags::preview(&source).expect("preview");
        let second_preview = FolderShuffleBags::preview(&source).expect("preview");
        assert_eq!(first_preview, second_preview);

        let id = ItemId(1);
        let mut bags = FolderShuffleBags::with_seed(7);
        let first = bags.resolve_turn(id, &directory.0).expect("first");
        let second = bags.resolve_turn(id, &directory.0).expect("second");
        assert_ne!(first, second);

        File::create(directory.0.join("new.jpeg")).expect("new fixture");
        let third = bags.resolve_turn(id, &directory.0).expect("rescan");
        assert!(matches!(
            third.file_name().and_then(|name| name.to_str()),
            Some("a.png" | "b.jpg" | "new.jpeg")
        ));
    }

    #[test]
    fn relative_asset_paths_reject_escape_and_absolute_paths() {
        assert!(is_safe_relative_asset_path(Path::new("assets/icon.png")));
        assert!(!is_safe_relative_asset_path(Path::new("../icon.png")));
        assert!(!is_safe_relative_asset_path(Path::new("C:\\icon.png")));
        assert!(!is_safe_relative_asset_path(Path::new("/icon.png")));
    }
}
