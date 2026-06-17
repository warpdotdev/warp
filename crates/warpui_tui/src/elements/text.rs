use crate::elements::TuiElement;
use crate::{TuiBuffer, TuiConstraint, TuiRect, TuiSize};

pub struct TuiText {
    text: String,
    size: TuiSize,
}

impl TuiText {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            size: TuiSize::default(),
        }
    }

    fn wrapped_lines(&self, width: u16) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }

        let width = usize::from(width);
        self.text
            .split('\n')
            .flat_map(|line| {
                if line.is_empty() {
                    return vec![String::new()];
                }

                line.chars()
                    .collect::<Vec<_>>()
                    .chunks(width)
                    .map(|chunk| chunk.iter().collect::<String>())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

impl TuiElement for TuiText {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        let width = constraint.max.width.max(constraint.min.width);
        let height = self
            .desired_height(width)
            .clamp(constraint.min.height, constraint.max.height);
        self.size = TuiSize::new(width, height);
        self.size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        for (row, line) in self
            .wrapped_lines(area.width)
            .into_iter()
            .take(usize::from(area.height))
            .enumerate()
        {
            let Ok(row) = u16::try_from(row) else {
                break;
            };
            buffer.write_str(area.x, area.y.saturating_add(row), area.width, &line);
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.wrapped_lines(width)
            .len()
            .try_into()
            .unwrap_or(u16::MAX)
    }
}
