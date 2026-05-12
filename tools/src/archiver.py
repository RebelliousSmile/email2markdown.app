from pathlib import Path
import shutil


def archive(filepath: Path, processed_dir: Path, delete: bool = False) -> None:
    """Move filepath to processed_dir, or delete if delete=True."""
    if delete:
        filepath.unlink()
    else:
        processed_dir.mkdir(parents=True, exist_ok=True)
        shutil.move(str(filepath), processed_dir / filepath.name)
