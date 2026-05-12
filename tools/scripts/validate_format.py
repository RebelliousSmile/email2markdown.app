import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

from src.parser import parse_email


def main() -> None:
    if len(sys.argv) != 2:
        print("Usage: python scripts/validate_format.py <folder_path>")
        sys.exit(1)

    folder = Path(sys.argv[1])
    if not folder.is_dir():
        print(f"Erreur : {folder} n'est pas un répertoire valide")
        sys.exit(1)

    md_files = sorted(folder.glob("*.md"))

    if not md_files:
        print("Avertissement : aucun fichier .md à traiter")
        return

    valid_count = 0

    for filepath in md_files:
        try:
            parse_email(filepath)
            valid_count += 1
        except ValueError as exc:
            print(f"INVALIDE: {filepath.name} — {exc}")

    print(f"OK: {valid_count} fichier(s) valide(s)")


if __name__ == "__main__":
    main()
