use std::path::Path;

use base64::Engine;
use pathfinder_geometry::vector::Vector2F;
use rand::Rng;
use warpui::util::parse_u32;

#[derive(Debug, Default, Clone)]
pub struct ITermImage {
    pub metadata: ITermImageMetadata,
    pub data: Vec<u8>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ITermImageDimensionUnit {
    Cell,
    Pixel,
    Percent,
}

#[derive(Debug, Clone)]
pub struct ITermImageMetadata {
    pub id: u32,
    pub desired_width: Option<(u32, ITermImageDimensionUnit)>,
    pub desired_height: Option<(u32, ITermImageDimensionUnit)>,
    pub preserve_aspect_ratio: bool,
    pub name: String,
    pub inline: bool,
    pub image_size: Vector2F,
}

impl Default for ITermImageMetadata {
    fn default() -> Self {
        Self {
            id: rand::thread_rng().gen(),
            desired_width: None,
            desired_height: None,
            preserve_aspect_ratio: true,
            name: "Unnamed file".into(),
            inline: false,
            image_size: Vector2F::default(),
        }
    }
}

fn parse_iterm_image_dimensions(dimension: &[u8]) -> Option<(u32, ITermImageDimensionUnit)> {
    if dimension.ends_with(b"px") {
        let value = &dimension[..dimension.len() - 2];
        Some((parse_u32(value)?, ITermImageDimensionUnit::Pixel))
    } else if dimension.ends_with(b"%") {
        let value = &dimension[..dimension.len() - 1];
        Some((
            parse_u32(value)?.clamp(0, 100),
            ITermImageDimensionUnit::Percent,
        ))
    } else {
        Some((parse_u32(dimension)?, ITermImageDimensionUnit::Cell))
    }
}

pub fn parse_iterm_image_metadata(params: &[&[u8]]) -> ITermImageMetadata {
    let mut metadata = ITermImageMetadata::default();
    for param in params {
        let (mut key, mut value) = match param.iter().position(|&byte| byte == b'=') {
            Some(position) => (&param[..position], &param[position + 1..]),
            None => continue,
        };

        // Because the format of arguments is (MultipartFile | File) = [optional arguments],
        // The first optional argument will have "MultipartFile=" or "File=" prefixed to it.
        // For example, params[1] will be "File=inline=0". So the key-value separation done before
        // will yield a value of "inline=0", which we need to further split.
        if key == b"File" || key == b"MultipartFile" {
            (key, value) = match value.iter().position(|&byte| byte == b'=') {
                Some(position) => (&value[..position], &value[position + 1..]),
                None => continue,
            };
        }

        // For unchunked File messages, image data will be contained in a param and will be after
        // a colon. So we need to remove it here to only look at the metadata.
        // For example. A param could be "height=100px:image_data".
        let value = match value.iter().position(|&byte| byte == b':') {
            Some(position) => &value[..position],
            None => value,
        };

        match key {
            b"width" => {
                metadata.desired_width = parse_iterm_image_dimensions(value);
            }
            b"height" => {
                metadata.desired_height = parse_iterm_image_dimensions(value);
            }
            b"preserveAspectRatio" => {
                metadata.preserve_aspect_ratio = value == b"1" || value == b"true";
            }
            b"name" => {
                let Ok(decoded_bytes) = base64::engine::general_purpose::STANDARD.decode(value)
                else {
                    continue;
                };
                let Ok(name) = String::from_utf8(decoded_bytes) else {
                    continue;
                };
                let Some(name) = Path::new(&name)
                    .file_name()
                    .and_then(|file_name| file_name.to_str())
                    .map(|file_name| file_name.to_string())
                else {
                    continue;
                };
                metadata.name = name;
            }
            b"inline" => {
                metadata.inline = value == b"1" || value == b"true";
            }
            _ => {}
        }
    }

    metadata
}

#[cfg(test)]
#[path = "iterm_image_tests.rs"]
mod tests;
