-- Makes the text field in the json table non-nullable and renames it to make it more generic.
DROP TABLE json_objects;

CREATE TABLE generic_string_objects (
    id INTEGER NOT NULL PRIMARY KEY,
    data TEXT NOT NULL
);
