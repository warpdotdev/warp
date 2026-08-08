import argparse
import hashlib
import os
import stat
from pathlib import Path


SEED_MARKER = ".warp-cache-seed.env"


def build_manifest(target_dir: Path) -> tuple[str, int]:
    digest = hashlib.sha256()
    file_count = 0

    for directory, directories, files in os.walk(target_dir):
        directories.sort()
        files.sort()
        directory_path = Path(directory)
        entries = [directory_path / filename for filename in files]
        entries.extend(
            directory_path / dirname
            for dirname in directories
            if (directory_path / dirname).is_symlink()
        )
        for path in sorted(entries):
            file_stat = path.lstat()
            relative_path_text = path.relative_to(target_dir).as_posix()
            if relative_path_text == SEED_MARKER:
                continue
            relative_path = relative_path_text.encode()
            file_kind = b"link" if stat.S_ISLNK(file_stat.st_mode) else b"file"
            metadata = (
                file_kind
                + b"\0"
                + str(file_stat.st_mode).encode()
                + b"\0"
                + str(file_stat.st_size).encode()
            )
            file_count += 1
            digest.update(len(relative_path).to_bytes(8, "big"))
            digest.update(relative_path)
            digest.update(len(metadata).to_bytes(8, "big"))
            digest.update(metadata)
            if stat.S_ISLNK(file_stat.st_mode):
                link_target = os.readlink(path).encode()
                digest.update(len(link_target).to_bytes(8, "big"))
                digest.update(link_target)
                continue
            if ".fingerprint" not in path.parts or not stat.S_ISREG(file_stat.st_mode):
                continue
            with path.open("rb") as fingerprint_file:
                while chunk := fingerprint_file.read(1024 * 1024):
                    digest.update(chunk)

    return digest.hexdigest(), file_count


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("target_dir", type=Path)
    args = parser.parse_args()

    manifest_sha, file_count = build_manifest(args.target_dir.resolve())
    print(manifest_sha, file_count)


if __name__ == "__main__":
    main()
