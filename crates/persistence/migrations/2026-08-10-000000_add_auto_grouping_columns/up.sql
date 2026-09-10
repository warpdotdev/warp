-- The project key a group is keyed by. NULL for manual groups, which is every
-- group that existed before automatic grouping.
ALTER TABLE tab_groups ADD COLUMN project_key TEXT;

-- Set while a tab is still waiting to be placed by automation. It tells a tab
-- the user deliberately ungrouped apart from one automation never reached
-- because its project key had not resolved yet.
ALTER TABLE tabs ADD COLUMN placed_by_automation BOOLEAN NOT NULL DEFAULT FALSE;
