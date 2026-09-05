#![allow(dead_code)]

use std::collections::HashMap;
use std::ops::{Not, RangeInclusive};
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::anyhow;
use rangemap::{RangeInclusiveMap, StepLite};
pub use secret_redaction::{
    RegexDisplayInfo, RegexLevelMetadata, SECRETS_REGEX, SecretLevel, SecretsRegex,
    find_secrets_in_text_with_levels_using_regex, merge_sorted_ranges_with_levels, regexes,
    set_user_and_enterprise_secret_regexes,
};

use super::RangeInModel;
use super::grid::grid_handler::GridHandler;
use super::grid::{Dimensions as _, RespectDisplayedOutput};
use crate::model::index::Point;

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd)]
/// A handle to a [`Secret`].
pub struct SecretHandle(usize);

impl SecretHandle {
    pub(super) fn next() -> Self {
        static SECRET_HANDLE: AtomicUsize = AtomicUsize::new(0);
        let next = SECRET_HANDLE.fetch_add(1, Ordering::Relaxed);
        SecretHandle(next)
    }

    pub fn id(&self) -> String {
        format!("{}", self.0)
    }
}

#[derive(Copy, Clone, Debug)]
pub enum IsObfuscated {
    Yes,
    No,
}

/// Whether or not to respect obfuscated secrets when retrieving grid contents.
#[derive(Copy, Clone, PartialEq)]
pub enum RespectObfuscatedSecrets {
    No,
    Yes,
}

/// Whether or not to obfuscate secrets during grid and tooltip rendering, respecting the Safe Mode setting.
#[derive(Clone, Copy, Debug, Default)]
pub enum ObfuscateSecrets {
    // Identify and visually obfuscate secrets
    Yes,
    /// Do not visually obfuscate secrets, but highlight them with a strikethrough
    Strikethrough,
    /// Show secrets with normal styling but still detect them for interaction (no visual treatment)
    AlwaysShow,
    #[default]
    No,
}

impl Not for ObfuscateSecrets {
    type Output = Self;

    fn not(self) -> Self::Output {
        match self {
            ObfuscateSecrets::Yes => ObfuscateSecrets::No,
            ObfuscateSecrets::No => ObfuscateSecrets::Yes,
            ObfuscateSecrets::Strikethrough => ObfuscateSecrets::Yes,
            ObfuscateSecrets::AlwaysShow => ObfuscateSecrets::Yes,
        }
    }
}

impl ObfuscateSecrets {
    /// Returns the "stronger" obfuscation mode. Priority: Yes > Strikethrough > AlwaysShow > No
    pub fn and(&self, other: &ObfuscateSecrets) -> ObfuscateSecrets {
        match (self, other) {
            (ObfuscateSecrets::Yes, _) | (_, ObfuscateSecrets::Yes) => ObfuscateSecrets::Yes,
            (ObfuscateSecrets::Strikethrough, _) | (_, ObfuscateSecrets::Strikethrough) => {
                ObfuscateSecrets::Strikethrough
            }
            (ObfuscateSecrets::AlwaysShow, _) | (_, ObfuscateSecrets::AlwaysShow) => {
                ObfuscateSecrets::AlwaysShow
            }
            (ObfuscateSecrets::No, ObfuscateSecrets::No) => ObfuscateSecrets::No,
        }
    }

    /// Returns whether the secret should be redacted given the current safe mode settings.
    /// This includes obfuscation, strikethrough, and always show (for interaction purposes).
    pub fn should_redact_secret(&self) -> bool {
        matches!(
            self,
            ObfuscateSecrets::Yes | ObfuscateSecrets::Strikethrough | ObfuscateSecrets::AlwaysShow
        )
    }

    /// Returns whether the current obfuscation mode is `ObfuscateSecrets::Yes`
    pub fn is_visually_obfuscated(&self) -> bool {
        matches!(self, ObfuscateSecrets::Yes)
    }
}

/// A secret (API key, password, etc) contained within the grid.
#[derive(Clone, Debug)]
pub struct Secret {
    /// Whether the secret is currently obfuscated.
    is_obfuscated: IsObfuscated,
    range: RangeInclusive<Point>,
    /// The level/source of this secret's redaction rule
    secret_level: SecretLevel,
}

impl RangeInModel for &Secret {
    fn range(&self) -> RangeInclusive<Point> {
        self.range.clone()
    }
}

impl RangeInModel for &mut Secret {
    fn range(&self) -> RangeInclusive<Point> {
        self.range.clone()
    }
}

pub type SecretAndHandle<'a> = (SecretHandle, &'a Secret);

impl Secret {
    pub(super) fn set_is_obfuscated(&mut self, is_obfuscated: IsObfuscated) {
        self.is_obfuscated = is_obfuscated
    }

    pub fn is_obfuscated(&self) -> bool {
        matches!(self.is_obfuscated, IsObfuscated::Yes)
    }

    pub fn new(
        is_obfuscated: IsObfuscated,
        range: RangeInclusive<Point>,
        secret_level: SecretLevel,
    ) -> Self {
        Self {
            is_obfuscated,
            range,
            secret_level,
        }
    }

    pub fn secret_level(&self) -> SecretLevel {
        self.secret_level
    }
}

/// Map that is responsible for storing secrets indexed by both [`SecretHandle`] and `Range`.
#[derive(Clone, Default, Debug)]
pub struct SecretMap {
    /// Mapping of secrets stored within the grid, keyed on the secret's [`SecretHandle`].
    secrets: HashMap<SecretHandle, Secret>,
    /// Mapping of secrets keyed on the range of the secret.
    secret_ranges: RangeInclusiveMap<RangeMapPoint, SecretHandle>,
}

impl SecretMap {
    /// Insert a [`Secret`] identified by `handle` into the map.
    pub fn insert(&mut self, handle: SecretHandle, secret: Secret, num_columns: usize) {
        let secret_range = secret.range.clone();
        let range_point_range = RangeMapPoint::new(*secret_range.start(), num_columns)
            ..=RangeMapPoint::new(*secret_range.end(), num_columns);
        self.secret_ranges.insert(range_point_range, handle);
        self.secrets.insert(handle, secret);
    }

    /// Removes a [`Secret`] identified by `handle` from the map.
    pub fn remove(&mut self, handle: SecretHandle, num_columns: usize) {
        let removed = self.secrets.remove(&handle);
        if let Some(secret) = removed {
            let range = RangeMapPoint::new(*secret.range.start(), num_columns)
                ..=RangeMapPoint::new(*secret.range.end(), num_columns);
            self.secret_ranges.remove(range);
        }
    }

    /// Returns the [`Secret`] identified by [`SecretHandle`] or `None` if no such secret exists.
    pub fn get_by_handle(&self, handle: &SecretHandle) -> Option<&Secret> {
        self.secrets.get(handle)
    }

    /// Returns the [`Secret`] and its corresponding [`SecretHandle`] contained at the current
    /// [`Point`]. Returns `None` if there is no secret at the given point.
    pub fn get_by_point(
        &self,
        point: Point,
        grid: &GridHandler,
        respect_displayed_output: RespectDisplayedOutput,
    ) -> Option<SecretAndHandle<'_>> {
        let original_point = if grid.has_displayed_output()
            && matches!(respect_displayed_output, RespectDisplayedOutput::Yes)
        {
            grid.maybe_translate_point_from_displayed_to_original(point)
        } else {
            point
        };
        let point_with_metadata = RangeMapPoint::new(original_point, grid.columns());
        let handle = self.secret_ranges.get(&point_with_metadata).copied();

        handle.zip(handle.and_then(|h| self.get_by_handle(&h)))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SecretHandle, &Secret)> {
        self.secrets.iter()
    }

    #[cfg(test)]
    pub fn ranges(&self) -> impl Iterator<Item = (RangeInclusive<Point>, &SecretHandle)> {
        self.secret_ranges
            .iter()
            .map(|(range, handle)| (range.start().as_point()..=range.end().as_point(), handle))
    }

    /// Clears all secrets within the map.
    pub fn clear(&mut self) {
        self.secrets.clear();
        self.secret_ranges.clear();
    }

    /// Marks the secret identified by `handle` as obfuscated. Returns an `Err` if no secret is
    /// identified by the `handle`.
    pub fn set_is_obfuscated(
        &mut self,
        handle: &SecretHandle,
        is_obfuscated: IsObfuscated,
    ) -> anyhow::Result<()> {
        let secret = self
            .secrets
            .get_mut(handle)
            .ok_or_else(|| anyhow!("No secret identified by provided SecretHandle"))?;
        secret.is_obfuscated = is_obfuscated;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.secrets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    /// Clears all of the secret ranges. Should be called after the resize of a grid since ranges
    /// are not stable across resizes.
    pub fn clear_ranges_after_resize(&mut self) {
        self.secret_ranges.clear();
    }
}

/// A wrapper around a [`Point`] that implements [`StepLite`], allowing us to store it in a
/// `RangeMap`. Used for secret redaction so we efficiently map from a given range to an underlying
/// secret stored at that range.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
struct RangeMapPoint {
    point: Point,
    num_cols: usize,
}

impl RangeMapPoint {
    fn new(point: Point, num_cols: usize) -> Self {
        Self { point, num_cols }
    }

    fn as_point(&self) -> Point {
        self.point
    }
}

impl StepLite for RangeMapPoint {
    fn add_one(&self) -> Self {
        let mut new_point = self.point;
        new_point.col += 1;
        if new_point.col >= self.num_cols {
            new_point.col = 0;
            new_point.row += 1;
        }

        RangeMapPoint {
            point: new_point,
            num_cols: self.num_cols,
        }
    }

    fn sub_one(&self) -> Self {
        let mut new_point = self.point;
        if new_point.col == 0 {
            if new_point.row == 0 {
                return *self;
            }
            new_point.row -= 1;
            new_point.col = self.num_cols - 1;
        } else {
            new_point.col -= 1;
        }

        RangeMapPoint {
            point: new_point,
            num_cols: self.num_cols,
        }
    }
}
