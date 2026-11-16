CREATE TABLE ignored_suggestions (
    id INTEGER NOT NULL PRIMARY KEY,
    suggestion TEXT NOT NULL,
    suggestion_type TEXT NOT NULL,
    UNIQUE(suggestion, suggestion_type)
);
