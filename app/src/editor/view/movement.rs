use anyhow::Result;
use warpui::AppContext;

use super::{DisplayMap, DisplayPoint};

pub fn left(
    map: &DisplayMap,
    mut point: DisplayPoint,
    app: &AppContext,
    stop_at_line_start: bool,
) -> Result<DisplayPoint> {
    if point.column() > 0 {
        *point.column_mut() -= 1;
    } else if !stop_at_line_start && point.row() > 0 {
        *point.row_mut() -= 1;
        *point.column_mut() = map.line_len(point.row(), app)?;
    }
    Ok(point)
}

pub fn right(
    map: &DisplayMap,
    mut point: DisplayPoint,
    app: &AppContext,
    stop_at_line_end: bool,
) -> Result<DisplayPoint> {
    let max_column = map.line_len(point.row(), app).unwrap();
    if point.column() < max_column {
        *point.column_mut() += 1;
    } else if !stop_at_line_end && point.row() < map.max_point(app).row() {
        *point.row_mut() += 1;
        *point.column_mut() = 0;
    }
    Ok(point)
}
