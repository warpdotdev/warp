pub mod undo_close;
pub use undo_close::UndoCloseView;

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        pub mod external_editor;
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "local_tty")] {
        pub mod startup_shell;
        pub use startup_shell::StartupShellView;

        pub mod working_directory;
        pub use working_directory::WorkingDirectoryView;
    }
}
