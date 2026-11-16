CREATE TABLE user_profiles (
    firebase_uid TEXT NOT NULL PRIMARY KEY,
    photo_url TEXT NOT NULL,
    email TEXT NOT NULL,
    display_name TEXT
);
