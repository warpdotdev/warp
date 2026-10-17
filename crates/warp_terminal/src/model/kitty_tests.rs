use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::*;

/// Reads `path` as a kitty `t=f` image on another thread, so that a read that never ends fails
/// the test rather than hanging it.
fn read_within_a_second(path: &Path) -> Result<Vec<u8>, InvalidKittyPayload> {
    let payload = path.to_str().expect("utf-8 path").as_bytes().to_vec();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(read_file(payload, false));
    });
    receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("the read should end")
}

#[test]
fn test_read_file_reads_a_regular_file_and_follows_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("image.png");
    fs::write(&image, b"png bytes").unwrap();
    assert_eq!(read_within_a_second(&image).unwrap(), b"png bytes");

    #[cfg(unix)]
    {
        let link = dir.path().join("link.png");
        std::os::unix::fs::symlink(&image, &link).unwrap();
        assert_eq!(read_within_a_second(&link).unwrap(), b"png bytes");
    }
}

#[cfg(unix)]
#[test]
fn test_read_file_refuses_all_but_regular_files_with_one_answer() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("fifo");
    nix::unistd::mkfifo(fifo.as_path(), nix::sys::stat::Mode::S_IRWXU).unwrap();
    let link_to_device = dir.path().join("zero");
    std::os::unix::fs::symlink("/dev/zero", &link_to_device).unwrap();
    // Sparse, so it takes no disk.
    let too_large = dir.path().join("large.png");
    fs::File::create(&too_large)
        .unwrap()
        .set_len(MAX_IMAGE_DATA_BYTES as u64 + 1)
        .unwrap();

    let refused = [
        fifo,
        link_to_device,
        too_large,
        dir.path().join("missing.png"),
        dir.path().to_owned(),
        "/dev/zero".into(),
    ]
    .into_iter()
    // A regular file, but in /proc.
    .chain(cfg!(target_os = "linux").then(|| "/proc/self/status".into()));

    let expected = format!(
        "{:?}",
        InvalidKittyPayload::FileError(FileError::FileReadError)
    );
    for path in refused {
        let reply = format!("{:?}", read_within_a_second(&path).unwrap_err());
        assert_eq!(reply, expected, "reading {path:?}");
    }
}

/// Puts `data` in a new shared-memory object `name`, as a program sending a `t=s` image does.
#[cfg(unix)]
fn create_shared_memory(name: &str, data: &[u8]) {
    use std::num::NonZero;

    use nix::fcntl::OFlag;
    use nix::sys::mman::{MapFlags, ProtFlags, mmap, munmap, shm_open};
    use nix::sys::stat::Mode;

    let fd = shm_open(
        name,
        OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_RDWR,
        Mode::S_IRUSR | Mode::S_IWUSR,
    )
    .unwrap();
    nix::unistd::ftruncate(fd, data.len() as i64).unwrap();
    let size = NonZero::new(data.len()).unwrap();
    unsafe {
        let ptr = mmap(
            None,
            size,
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_SHARED,
            fd,
            0,
        )
        .unwrap();
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
        munmap(ptr, data.len()).unwrap();
    }
    nix::unistd::close(fd).unwrap();
}

#[cfg(unix)]
#[test]
fn test_read_shared_memory_closes_what_it_opens() {
    let open_fds = || fs::read_dir("/dev/fd").unwrap().count();
    let control_data = parse_kitty_control_data(b"f=32,s=2,v=1,t=s");
    let before = open_fds();

    for n in 0..500 {
        let name = format!("/wkt-{}-{n}", std::process::id());
        create_shared_memory(&name, &[7; 8]);
        let data = read_shared_memory(control_data.clone(), name.into_bytes()).unwrap();
        assert_eq!(data, [7; 8]);
    }

    // A read that kept its object open would leave 500 more; other tests may hold a few.
    assert!(open_fds() < before + 100);
}

#[cfg(unix)]
#[test]
fn test_read_shared_memory_refuses_images_over_the_cap() {
    let name = format!("/wkt-large-{}", std::process::id());
    create_shared_memory(&name, &[0; 4]);
    // 4 bytes for each of u32::MAX × u32::MAX pixels: more than a usize holds.
    let control_data =
        parse_kitty_control_data(format!("f=32,s={0},v={0},t=s", u32::MAX).as_bytes());

    assert!(matches!(
        read_shared_memory(control_data, name.into_bytes()),
        Err(InvalidKittyPayload::ShmError(ShmError::InvalidObjectSize))
    ));
}
