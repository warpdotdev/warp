-- migrate the dataset to use the new coordinate system conversion: before the change,
-- we convert coordinates with "origin_y - window_height". After the change, we convert
-- the coordinates with "-(origin_y + window_height)". The delta defined here makes
-- sure the coordinate will be valid after the new conversion
-- "-(origin_y - window_height) - 2 * window_height = -(origin_y + window_height)".
UPDATE windows SET origin_y = -origin_y - 2 * window_height WHERE origin_y IS NOT NULL;
