# APP-4386 — SSH Remote Server Install Fallback (wget + SCP)
Linear: [APP-4386](https://linear.app/warpdotdev/issue/APP-4386)

## Context
When a user SSHes into a remote host, the client installs the remote server binary by piping `install_remote_server.sh` through `bash -s` on the remote. The script unconditionally uses `curl` (line 43) to download the tarball. On minimal hosts (Alpine, BusyBox, stripped Docker images), `curl` is absent → `bash: line 43: curl: command not found` (exit 127), and the install fails with no recovery path.

Other remote-development editors solve this with a multi-tier fallback strategy: try `curl` on the remote → fall back to `wget` → fall back to downloading locally and uploading via SCP. Warp currently has a single tier: curl only, no fallback.

### Relevant code
- `crates/remote_server/src/install_remote_server.sh` — the install script; line 43 is the sole `curl` invocation
- `crates/remote_server/src/setup.rs:385-414` — `INSTALL_SCRIPT_TEMPLATE` loaded via `include_str!`; `install_script()` substitutes placeholders (`{download_base_url}`, `{channel}`, `{install_dir}`, `{binary_name}`, `{version_query}`, `{version_suffix}`)
- `crates/remote_server/src/setup.rs:416-441` — `download_url()` and `download_channel()` construct the full CDN URL
- `app/src/remote_server/ssh_transport.rs:194-217` — `SshTransport::install_binary()` runs the script via `run_ssh_script` and surfaces success/failure
- `crates/remote_server/src/transport.rs:117-127` — `RemoteTransport::install_binary` trait method; returns `Result<(), String>`
- `crates/remote_server/src/ssh.rs:95-155` — `run_ssh_command` and `run_ssh_script` utilities
- `crates/remote_server/src/manager.rs:596-646` — `RemoteServerManager::install_binary` orchestrates the install, emits `SetupStateChanged` and `BinaryInstallComplete`
- `crates/remote_server/src/setup.rs:202-237` — `RemotePlatform`, `RemoteOs`, `RemoteArch` — already detected before install via `detect_platform`

## Proposed changes
Two phases, both in this PR.

### Phase 1: wget fallback in the shell script
Modify `install_remote_server.sh` to detect which HTTP client is available and use whichever is present. The download URL construction stays identical — only the download command changes.

Replace the current `curl` invocation (lines 43-44):
```bash
curl -fSL "{download_base_url}?package=tar&os=$os_name&arch=$arch_name&channel={channel}{version_query}" \
  -o "$tmpdir/oz.tar.gz"
```

With a detection block:
```bash
url="{download_base_url}?package=tar&os=$os_name&arch=$arch_name&channel={channel}{version_query}"

if command -v curl >/dev/null 2>&1; then
  curl -fSL "$url" -o "$tmpdir/oz.tar.gz"
elif command -v wget >/dev/null 2>&1; then
  wget -q -O "$tmpdir/oz.tar.gz" "$url"
else
  echo "error: neither curl nor wget is available" >&2
  exit 3
fi
```

The exit code for "no HTTP client" is shared as a constant in `setup.rs` and injected into the script via placeholder substitution:
```rust
/// Exit code the install script uses when neither curl nor wget is
/// available on the remote host. The Rust side matches on this to
/// trigger the SCP upload fallback.
pub const NO_HTTP_CLIENT_EXIT_CODE: i32 = 3;
```
The script template uses `exit {no_http_client_exit_code}` instead of a hardcoded `exit 3`, and `install_script()` substitutes it alongside the existing placeholders.

Key details:
- `command -v` is POSIX-compliant and works on BusyBox `sh`, `dash`, `bash`, and `zsh`. Preferred over `which` (non-POSIX, absent on some minimal systems) and `type` (output format varies across shells).
- Exit code 3 is the next unused code after exit 1 (no binary in tarball) and exit 2 (unsupported arch/OS).
- `wget -q -O` matches the semantics of `curl -fSL -o`: quiet output, write to a specific file, follow redirects (wget follows by default, up to 20 hops). The `-f` (fail on HTTP errors) has no direct wget equivalent, but wget exits non-zero on 4xx/5xx by default.
- The `url` variable is extracted to avoid duplicating the long URL string between the curl and wget branches.
- The `{placeholder}` substitution from `setup.rs` is unchanged — no Rust changes needed for Phase 1 beyond the new constant and placeholder.

### Phase 2: SCP upload fallback in Rust
When the install script exits with code 3 (no HTTP client), the client downloads the tarball locally and uploads it to the remote via `scp` through the existing ControlMaster socket.

This requires changes across three layers:

#### 2a. New `download_url()` and `install_tarball_path()` public helpers in `setup.rs`
Expose the download URL construction so the Rust-side SCP fallback can download the same tarball the shell script would have fetched.

```rust
/// Returns the full download URL for the remote server tarball,
/// parameterized by the remote platform.
pub fn download_tarball_url(platform: &RemotePlatform) -> String {
    format!(
        "{}?package=tar&os={}&arch={}&channel={}{}",
        download_url(),
        platform.os.as_str(),
        platform.arch.as_str(),
        download_channel(),
        version_query(),
    )
}

/// Returns the remote path where the tarball should be uploaded
/// before the extraction script runs.
pub fn remote_tarball_staging_path() -> String {
    format!("{}/oz-upload.tar.gz", remote_server_dir())
}
```

Also extract a `version_query()` helper (currently inlined in `install_script()`) so both the shell script and the Rust download path use the same query string.

#### 2b. New `scp_upload` utility in `ssh.rs`
Add an `scp` helper that uploads a local file to the remote through the ControlMaster socket:

```rust
/// Upload a local file to the remote host via `scp`, reusing the
/// ControlMaster socket for authentication. Returns `Ok(())` on
/// success or an error describing the failure.
pub async fn scp_upload(
    socket_path: &Path,
    local_path: &Path,
    remote_path: &str,
    timeout: Duration,
) -> Result<()> {
    async {
        Command::new("scp")
            .arg("-o").arg(format!("ControlPath={}", socket_path.display()))
            .arg("-o").arg("ControlMaster=no")
            .arg("-o").arg("ConnectTimeout=15")
            .arg(local_path.as_os_str())
            .arg(format!("placeholder@placeholder:{remote_path}"))
            .kill_on_drop(true)
            .output()
            .await
    }
    .with_timeout(timeout)
    .await
    .map_err(|_| anyhow!("scp timed out after {timeout:?}"))?
    .map_err(|e| anyhow!("scp failed to execute: {e}"))
    .and_then(|output| {
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow!("scp failed (exit {:?}): {stderr}", output.status.code()))
        }
    })
}
```

Key details:
- Reuses the same ControlMaster socket (`-o ControlPath=...`) that all other SSH operations use — no re-authentication needed.
- `placeholder@placeholder` matches the convention in `ssh_args()` (`ssh.rs:26`): the ControlMaster socket already has the real user/host baked in, so the CLI arguments are placeholders.
- `ControlMaster=no` ensures scp joins the existing master session rather than trying to become one.
- Timeout uses `INSTALL_TIMEOUT` (60s) from the caller since the upload replaces the download step.

#### 2c. Shared extraction logic — single script with a tarball-path argument
Rather than maintaining two scripts with duplicated extraction code, refactor `install_remote_server.sh` so the download and extraction phases are cleanly separated within the same file. The script accepts an optional `$1` argument: a path to an already-uploaded tarball. When provided, the script skips the download phase entirely and extracts from that path. When omitted, it runs the curl/wget download as before.

```bash
if [ -n "$1" ]; then
  # SCP fallback: tarball already uploaded by the client.
  tarball_src="$1"
  mv "$tarball_src" "$tmpdir/oz.tar.gz"
else
  # Normal path: download via curl or wget.
  url="{download_base_url}?package=tar&os=$os_name&arch=$arch_name&channel={channel}{version_query}"
  if command -v curl >/dev/null 2>&1; then
    curl -fSL "$url" -o "$tmpdir/oz.tar.gz"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$tmpdir/oz.tar.gz" "$url"
  else
    echo "error: neither curl nor wget is available" >&2
    exit {no_http_client_exit_code}
  fi
fi

# Shared extraction tail (unchanged from today's lines 45-50).
tar -xzf "$tmpdir/oz.tar.gz" -C "$tmpdir"
bin=$(find "$tmpdir" -type f -name 'oz*' ! -name '*.tar.gz' | head -n1)
if [ -z "$bin" ]; then echo "no binary found in tarball" >&2; exit 1; fi
chmod +x "$bin"
mv "$bin" "$install_dir/{binary_name}{version_suffix}"
```

The SCP fallback in Rust invokes the same script with the staging path as `$1` via `run_ssh_script` by passing `bash -s -- <staging_path>` (or equivalently prepending the argument to the script). This eliminates code duplication and ensures any future extraction changes (e.g. checksum verification) apply to both paths.

The `{staging_path}` used by the SCP fallback is the expanded form of `remote_tarball_staging_path()`.

#### 2d. Modify `SshTransport::install_binary` to orchestrate the fallback
Change `install_binary` in `ssh_transport.rs` from a single `run_ssh_script` call to a two-step flow:

1. Run the existing install script (now with wget fallback from Phase 1).
2. If the script exits with code 3, fall back to SCP:
   a. Build the download URL using `download_tarball_url` with the `RemotePlatform` already detected by `detect_platform` (called earlier in the setup flow).
   b. Download the tarball locally using the system's HTTP client (reqwest or a `curl` subprocess on the local machine — the local machine is guaranteed to have internet access).
   c. Upload via `scp_upload` to the staging path.
   d. Run the extraction-only script via `run_ssh_script`.

To support this, `SshTransport` needs access to the `RemotePlatform` detected earlier. Add it as a field set during construction or via a setter called by the controller after `detect_platform` succeeds (it's already called before `install_binary` in the setup flow at `manager.rs:500-530`).

```rust
fn install_binary(&self) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
    let socket_path = self.socket_path.clone();
    let platform = self.platform.clone();
    Box::pin(async move {
        let script = remote_server::setup::install_script();
        match remote_server::ssh::run_ssh_script(
            &socket_path, &script, remote_server::setup::INSTALL_TIMEOUT,
        ).await {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) if output.status.code() == Some(remote_server::setup::NO_HTTP_CLIENT_EXIT_CODE) => {
                // No HTTP client on remote — fall back to local download + SCP.
                log::info!("Remote has no curl/wget, falling back to SCP upload");
                let Some(platform) = platform else {
                    return Err("SCP fallback requires platform detection".into());
                };
                scp_install_fallback(&socket_path, &platform).await
            }
            Ok(output) => {
                let code = output.status.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!("install script failed (exit {code}): {stderr}"))
            }
            Err(e) => Err(format!("{e:#}")),
        }
    })
}
```

The `scp_install_fallback` helper:
1. Constructs the URL via `download_tarball_url(&platform)`.
2. Creates a local temp directory (`tempfile::tempdir()`) and downloads the tarball into it via a local `curl` subprocess. The temp directory is cleaned up automatically when the `TempDir` guard drops (including on early-return errors).
3. Calls `scp_upload` to transfer the local tarball to the remote staging path.
4. Re-invokes the install script with the staging path as `$1` so the shared extraction tail runs.
5. `TempDir` drop handles cleanup.

#### Error handling and timeout budget
The SCP fallback uses a separate, longer timeout:
```rust
/// Timeout for the SCP upload fallback path (local download + SCP + extraction).
/// Longer than `INSTALL_TIMEOUT` because SCP transfers the ~30-50 MB tarball
/// over the user's SSH link, which is typically slower than the remote host's
/// direct internet connection. On a 1 MB/s link, upload alone takes 30-50s.
pub const SCP_INSTALL_TIMEOUT: Duration = Duration::from_secs(120);
```
The standard `INSTALL_TIMEOUT` (60s) is sufficient for the curl/wget path because the remote host downloads directly from the CDN. The SCP path adds a local download step (~5s) plus an SCP upload that depends entirely on the SSH link bandwidth — embedded devices, VPNs, and high-latency connections can easily exceed 60s for a ~30-50 MB transfer. Each sub-step (`scp_upload`, `run_ssh_script` for extraction) uses the full `SCP_INSTALL_TIMEOUT` individually to avoid splitting a single budget across steps, which would require coordination logic for diminishing remaining time.
- If the SCP upload or extraction fails, the error surfaces the same way as a normal install failure — `BinaryInstallComplete { result: Err(_) }` — and the session falls back to ControlMaster warpification.

## Testing and validation

### Unit tests (`setup_tests.rs`)
- **`install_script_contains_wget_fallback`** — Assert that `install_script()` output contains `command -v curl`, `command -v wget`, and the `exit {no_http_client_exit_code}` sentinel. This is the main guardrail against accidentally regressing the fallback logic during future script edits.
- **`download_tarball_url_formats_correctly`** — Test `download_tarball_url` for each `(RemoteOs, RemoteArch)` combination. Catches URL construction drift between the shell script placeholders and the Rust-side URL builder, which would cause the SCP fallback to download the wrong artifact.

### Integration / manual testing
- **No curl, has wget**: SSH into a Docker container with `apt-get remove curl` / Alpine with only `wget`. Verify install succeeds via wget path.
- **No curl, no wget**: SSH into a minimal BusyBox container with neither. Verify the script exits with `NO_HTTP_CLIENT_EXIT_CODE`, the client downloads locally, SCPs the tarball, and the extraction script installs successfully.
- **Happy path unchanged**: SSH into a standard Ubuntu/Debian host. Verify curl is still used (check logs for absence of "falling back" message).

## Risks and mitigations
- **BusyBox `wget` differences**: BusyBox's `wget` is a stripped-down implementation that lacks some GNU wget flags. The flags used (`-q -O`) are supported by BusyBox wget. Notably, BusyBox wget does NOT support `--connect-timeout` — we omit it and rely on the outer SSH timeout (60s) instead.
- **SCP deprecation**: OpenSSH has been moving toward SFTP as the default transfer protocol. Modern `scp` (OpenSSH 9.0+) uses the SFTP protocol under the hood by default, so this isn't a practical concern. If a host has a very old SSH that lacks SCP, it almost certainly has curl or wget.
- **Local download assumes curl on client**: The local machine (macOS, Linux desktop, or Windows with WSL) is expected to have `curl`. macOS ships curl, and it's near-universal on Linux desktops.

## Parallelization
This is a small, focused change. Sub-agents are not beneficial — the shell script change (Phase 1) and the Rust SCP fallback (Phase 2) are tightly coupled and should be in the same PR.
