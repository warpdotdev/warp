ALTER TABLE windows ADD warp_ai_width FLOAT CHECK (warp_ai_width >= 0);
ALTER TABLE windows ADD voltron_width FLOAT CHECK (voltron_width >= 0);
ALTER TABLE windows ADD warp_drive_index_width FLOAT CHECK (warp_drive_index_width >= 0);
ALTER TABLE windows ADD warp_drive_asset_width FLOAT CHECK (warp_drive_asset_width >= 0);
