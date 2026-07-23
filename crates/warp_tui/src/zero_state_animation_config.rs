use std::fmt;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use warp::settings::{TuiZeroStateObject, TuiZeroStateSettings, TuiZeroStateSettingsChangedEvent};
use warp_core::safe_warn;
use warp_core::settings::Setting;
use warpui::SingletonEntity;
use warpui_core::{AppContext, Entity};

use super::{
    BUILT_IN_LOGO_CELL_ASPECT_RATIO, MAX_ASCII_ART_BYTES, MAX_ASCII_ART_COLS, MAX_ASCII_ART_ROWS,
    warp_logo_contains,
};

#[derive(Clone, Debug)]
pub(super) enum ZeroStateShape {
    BuiltInWarp,
    Ascii(AsciiArtMask),
}

impl ZeroStateShape {
    pub(super) fn contains(&self, x: f64, y: f64) -> bool {
        match self {
            Self::BuiltInWarp => warp_logo_contains(x, y),
            Self::Ascii(mask) => mask.contains(x, y),
        }
    }

    pub(super) fn cell_aspect_ratio(&self) -> f64 {
        match self {
            Self::BuiltInWarp => BUILT_IN_LOGO_CELL_ASPECT_RATIO,
            Self::Ascii(mask) => mask.cell_aspect_ratio(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AsciiArtMask {
    width: usize,
    height: usize,
    cells: Vec<bool>,
}

impl AsciiArtMask {
    pub(super) fn parse(source: &str) -> Result<Self, AsciiArtError> {
        if source.len() as u64 > MAX_ASCII_ART_BYTES {
            return Err(AsciiArtError::TooLarge {
                bytes: source.len() as u64,
            });
        }

        let normalized = source.replace("\r\n", "\n");
        if normalized.contains('\r') {
            return Err(AsciiArtError::InvalidCharacter);
        }
        let mut rows = normalized.split('\n').collect::<Vec<_>>();
        if rows.last() == Some(&"") {
            rows.pop();
        }
        if rows.len() > MAX_ASCII_ART_ROWS {
            return Err(AsciiArtError::TooManyRows { rows: rows.len() });
        }

        let mut min_x = usize::MAX;
        let mut max_x = 0;
        let mut min_y = usize::MAX;
        let mut max_y = 0;
        let mut found_occupied = false;
        for (y, row) in rows.iter().enumerate() {
            if row.len() > MAX_ASCII_ART_COLS {
                return Err(AsciiArtError::TooManyColumns { columns: row.len() });
            }
            for (x, byte) in row.bytes().enumerate() {
                if !(b' '..=b'~').contains(&byte) {
                    return Err(AsciiArtError::InvalidCharacter);
                }
                if byte != b' ' {
                    found_occupied = true;
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        if !found_occupied {
            return Err(AsciiArtError::Empty);
        }

        let width = max_x - min_x + 1;
        let height = max_y - min_y + 1;
        let mut cells = vec![false; width * height];
        for source_y in min_y..=max_y {
            let row = rows[source_y].as_bytes();
            for source_x in min_x..=max_x {
                cells[(source_y - min_y) * width + source_x - min_x] =
                    row.get(source_x).is_some_and(|byte| *byte != b' ');
            }
        }
        Ok(Self {
            width,
            height,
            cells,
        })
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        if !(-1.0..=1.0).contains(&x) || !(-1.0..=1.0).contains(&y) {
            return false;
        }
        let column = (((x + 1.0) * 0.5 * self.width as f64).floor() as usize).min(self.width - 1);
        let row = (((y + 1.0) * 0.5 * self.height as f64).floor() as usize).min(self.height - 1);
        self.cells[row * self.width + column]
    }

    fn cell_aspect_ratio(&self) -> f64 {
        self.width as f64 / self.height as f64
    }

    #[cfg(test)]
    pub(super) fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }
}

#[derive(Debug)]
pub(super) enum AsciiArtError {
    Io(std::io::Error),
    NotAFile,
    TooLarge { bytes: u64 },
    TooManyColumns { columns: usize },
    TooManyRows { rows: usize },
    InvalidCharacter,
    Empty,
}

impl fmt::Display for AsciiArtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::NotAFile => write!(f, "path is not a regular file"),
            Self::TooLarge { bytes } => {
                write!(f, "file is {bytes} bytes; limit is {MAX_ASCII_ART_BYTES}")
            }
            Self::TooManyColumns { columns } => {
                write!(
                    f,
                    "art has {columns} columns; limit is {MAX_ASCII_ART_COLS}"
                )
            }
            Self::TooManyRows { rows } => {
                write!(f, "art has {rows} rows; limit is {MAX_ASCII_ART_ROWS}")
            }
            Self::InvalidCharacter => {
                write!(f, "art must contain only printable ASCII and newlines")
            }
            Self::Empty => write!(f, "art does not contain any occupied cells"),
        }
    }
}

impl From<std::io::Error> for AsciiArtError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ZeroStateAnimationConfig {
    pub(super) active_object: TuiZeroStateObject,
    pub(super) shape: Arc<ZeroStateShape>,
    pub(super) rotation_period: Duration,
    pub(super) extrusion_depth: f64,
    pub(super) load_failure: Option<ZeroStateAnimationLoadFailure>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ZeroStateAnimationLoadFailure {
    InitialLoad,
    Reload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ZeroStateAnimationConfigEvent {
    Updated,
    LoadFailed(ZeroStateAnimationLoadFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReloadObjectOutcome {
    Unchanged,
    Reloaded,
    Failed,
}

impl Default for ZeroStateAnimationConfig {
    fn default() -> Self {
        Self {
            active_object: TuiZeroStateObject::BuiltIn,
            shape: Arc::new(ZeroStateShape::BuiltInWarp),
            rotation_period: Duration::from_secs_f64(
                warp::settings::DEFAULT_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS,
            ),
            extrusion_depth: warp::settings::DEFAULT_TUI_ZERO_STATE_EXTRUSION_DEPTH,
            load_failure: None,
        }
    }
}

impl Entity for ZeroStateAnimationConfig {
    type Event = ZeroStateAnimationConfigEvent;
}

impl SingletonEntity for ZeroStateAnimationConfig {}

impl ZeroStateAnimationConfig {
    pub(crate) fn register(ctx: &mut AppContext) {
        let config_dir = warp_core::paths::tui_config_local_dir();
        let (object, rotation_period, extrusion_depth) = {
            let settings = TuiZeroStateSettings::as_ref(ctx);
            (
                settings.object.value().clone(),
                settings.rotation_period_seconds.value().get(),
                settings.extrusion_depth.value().get(),
            )
        };
        let config = Self::load(&object, rotation_period, extrusion_depth, &config_dir);
        let config = ctx.add_singleton_model(move |_| config);
        ctx.subscribe_to_model(
            &TuiZeroStateSettings::handle(ctx),
            move |settings, event, ctx| {
                let TuiZeroStateSettingsChangedEvent::TuiZeroStateObjectSetting { .. } = event
                else {
                    return;
                };
                let object = settings.as_ref(ctx).object.value().clone();
                config.update(ctx, |config, ctx| {
                    match config.reload_object(&object, &config_dir) {
                        ReloadObjectOutcome::Unchanged => {}
                        ReloadObjectOutcome::Reloaded => {
                            ctx.emit(ZeroStateAnimationConfigEvent::Updated);
                        }
                        ReloadObjectOutcome::Failed => {
                            ctx.emit(ZeroStateAnimationConfigEvent::LoadFailed(
                                ZeroStateAnimationLoadFailure::Reload,
                            ));
                        }
                    }
                });
            },
        );
    }

    pub(crate) fn load(
        object: &TuiZeroStateObject,
        rotation_period_seconds: f64,
        extrusion_depth: f64,
        config_dir: &Path,
    ) -> Self {
        let (active_object, shape, load_failure) = match load_object_shape(object, config_dir) {
            Ok(shape) => (object.clone(), shape, None),
            Err((path, error)) => {
                safe_warn!(
                    safe: (
                        "Could not load custom Warp Agent CLI zero-state ASCII art; using the built-in Warp logo"
                    ),
                    full: (
                        "Could not load custom Warp Agent CLI zero-state ASCII art; using the built-in Warp logo: path={} error={error}",
                        path.display()
                    )
                );
                (
                    TuiZeroStateObject::BuiltIn,
                    ZeroStateShape::BuiltInWarp,
                    Some(ZeroStateAnimationLoadFailure::InitialLoad),
                )
            }
        };
        Self {
            active_object,
            shape: Arc::new(shape),
            rotation_period: Duration::from_secs_f64(rotation_period_seconds),
            extrusion_depth,
            load_failure,
        }
    }

    pub(crate) fn load_failure(&self) -> Option<ZeroStateAnimationLoadFailure> {
        self.load_failure
    }

    pub(super) fn reload_object(
        &mut self,
        object: &TuiZeroStateObject,
        config_dir: &Path,
    ) -> ReloadObjectOutcome {
        if &self.active_object == object {
            self.load_failure = None;
            return ReloadObjectOutcome::Unchanged;
        }

        match load_object_shape(object, config_dir) {
            Ok(shape) => {
                self.active_object = object.clone();
                self.shape = Arc::new(shape);
                self.load_failure = None;
                ReloadObjectOutcome::Reloaded
            }
            Err((path, error)) => {
                safe_warn!(
                    safe: (
                        "Could not reload custom Warp Agent CLI zero-state ASCII art; keeping the current object"
                    ),
                    full: (
                        "Could not reload custom Warp Agent CLI zero-state ASCII art; keeping the current object: path={} error={error}",
                        path.display()
                    )
                );
                self.load_failure = Some(ZeroStateAnimationLoadFailure::Reload);
                ReloadObjectOutcome::Failed
            }
        }
    }
}

pub(super) fn resolve_ascii_art_path(path: &Path, config_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        config_dir.join(path)
    }
}

fn load_object_shape(
    object: &TuiZeroStateObject,
    config_dir: &Path,
) -> Result<ZeroStateShape, (PathBuf, AsciiArtError)> {
    match object {
        TuiZeroStateObject::BuiltIn => Ok(ZeroStateShape::BuiltInWarp),
        TuiZeroStateObject::AsciiFile { path } => {
            let path = resolve_ascii_art_path(path, config_dir);
            load_ascii_art(&path)
                .map(ZeroStateShape::Ascii)
                .map_err(|error| (path, error))
        }
    }
}
fn load_ascii_art(path: &Path) -> Result<AsciiArtMask, AsciiArtError> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(AsciiArtError::NotAFile);
    }
    if metadata.len() > MAX_ASCII_ART_BYTES {
        return Err(AsciiArtError::TooLarge {
            bytes: metadata.len(),
        });
    }
    let mut source = String::new();
    (&mut file)
        .take(MAX_ASCII_ART_BYTES + 1)
        .read_to_string(&mut source)?;
    AsciiArtMask::parse(&source)
}
