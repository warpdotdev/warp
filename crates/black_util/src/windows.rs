//! Windows-specific utilities.

/// Attaches the current process to the console of the parent process.
///
/// This is useful for command-line interfaces that need to ensure all standard
/// output gets printed correctly when run from a terminal.
pub fn attach_to_parent_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}
