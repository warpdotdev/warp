use crate::model::Point;

impl From<Point> for session_sharing_protocol::common::Point {
    fn from(val: Point) -> Self {
        session_sharing_protocol::common::Point {
            row: val.row,
            col: val.col,
        }
    }
}

impl From<session_sharing_protocol::common::Point> for Point {
    fn from(value: session_sharing_protocol::common::Point) -> Self {
        Self {
            row: value.row,
            col: value.col,
        }
    }
}
