use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        macos: { target_os = "macos" },
        linux: { any(target_os = "linux", target_os = "freebsd") },
        noop: { not(any(macos, linux, windows)) },
    }
}
