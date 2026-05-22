use std::borrow::Cow;
use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use warpui::color::ColorU;

use super::OPAQUE;

const SHORT_COLOR_LEN: usize = 3;
const FULL_COLOR_LEN: usize = 6;

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum HexColorError {
    HashPrefix,
    InvalidLength,
    InvalidValue,
}

impl fmt::Display for HexColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HexColorError::HashPrefix => {
                write!(f, "Expected hex color string starting with #.")
            }
            HexColorError::InvalidLength => write!(
                f,
                "Expected hex color string starting with # followed by 3 or 6 characters."
            ),
            HexColorError::InvalidValue => write!(f, "Invalid hex color string"),
        }
    }
}

pub fn coloru_from_hex_string(s: &str) -> Result<ColorU, HexColorError> {
    if !s.starts_with('#') {
        return Err(HexColorError::HashPrefix);
    }
    let mut s: Cow<str> = s[1..].into();

    if s.len() != SHORT_COLOR_LEN && s.len() != FULL_COLOR_LEN {
        return Err(HexColorError::InvalidLength);
    }

    // for a shorter color representation we want to "normalize" it to the standard 6-character
    // one, so #123 becomes #112233.
    if s.len() == SHORT_COLOR_LEN {
        s = s
            .chars()
            .flat_map(|c| std::iter::repeat_n(c, 2))
            .collect::<String>()
            .into();
    }

    let parsed = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>();

    match parsed {
        Ok(bytes) if bytes.len() == 3 => Ok(ColorU {
            r: bytes[0],
            g: bytes[1],
            b: bytes[2],
            a: OPAQUE,
        }),
        _ => Err(HexColorError::InvalidValue),
    }
}

pub fn coloru_to_hex_string(coloru: &ColorU) -> String {
    format!("#{:02x}{:02x}{:02x}", coloru.r, coloru.g, coloru.b)
}

pub fn deserialize<'de, D, C>(deserializer: D) -> Result<C, D::Error>
where
    C: From<ColorU>,
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    coloru_from_hex_string(&s)
        .map(Into::into)
        .map_err(de::Error::custom)
}

pub fn serialize<S, C>(color: &C, serializer: S) -> Result<S::Ok, S::Error>
where
    C: Into<ColorU> + Clone,
    S: Serializer,
{
    let coloru: ColorU = color.to_owned().into();
    coloru_to_hex_string(&coloru).serialize(serializer)
}
