pub mod model;
pub mod schema;

#[cfg(feature = "local_fs")]
pub const MIGRATIONS: diesel_migrations::EmbeddedMigrations =
    diesel_migrations::embed_migrations!("migrations");
