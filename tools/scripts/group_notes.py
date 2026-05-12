"""Concatenate selected email-markdown notes into a single grouped file.

Usage:
    group_notes.py --input-files <list> --output <path>

Each input file's YAML frontmatter is preserved as a section header in the
output. A minimal aggregated frontmatter is emitted at the top with:
    title:       (derived from the first subject, or "Groupe")
    sources:     (count + list of original filenames)
    start_date / end_date (min/max of frontmatter `date` fields)

Bodies are concatenated with `---` separators. The script never touches the
source files — caller decides whether to delete or archive them.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

if sys.stdout.encoding and sys.stdout.encoding.lower() != "utf-8":
    import io
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
if sys.stderr.encoding and sys.stderr.encoding.lower() != "utf-8":
    import io
    sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", errors="replace")

import argparse
import logging

import frontmatter
import yaml

logging.basicConfig(level=logging.WARNING, format="%(levelname)s: %(message)s")
logger = logging.getLogger(__name__)


def _load(filepath: Path):
    post = frontmatter.load(filepath)
    return dict(post.metadata or {}), post.content, filepath.name


def main() -> None:
    parser = argparse.ArgumentParser(description="Concatenate email notes into a single grouped markdown file.")
    parser.add_argument("--input-files", nargs="+", required=True, help="Input .md files (>=2)")
    parser.add_argument("--output", required=True, help="Output file path")
    args = parser.parse_args()

    items = []
    for raw in args.input_files:
        fp = Path(raw)
        if not fp.exists():
            logger.warning("Fichier ignoré (absent) : %s", fp)
            continue
        try:
            items.append(_load(fp))
        except Exception as exc:
            logger.warning("Parse échoué %s : %s", fp.name, exc)

    if not items:
        print("Aucun fichier valide à grouper.", file=sys.stderr)
        sys.exit(1)

    items.sort(key=lambda it: str(it[0].get("date", "") or ""))

    first_meta = items[0][0]
    dates = [str(m.get("date", "") or "") for m, _, _ in items if m.get("date")]
    aggregated = {
        "title": first_meta.get("subject") or "Groupe",
        "sources": [name for _, _, name in items],
        "source_count": len(items),
    }
    if dates:
        aggregated["start_date"] = min(dates)
        aggregated["end_date"] = max(dates)

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    with out_path.open("w", encoding="utf-8") as fh:
        fh.write("---\n")
        yaml.safe_dump(aggregated, fh, allow_unicode=True, sort_keys=False)
        fh.write("---\n\n")
        for idx, (meta, body, name) in enumerate(items):
            subject = meta.get("subject") or name
            date = meta.get("date") or ""
            sender = meta.get("from") or meta.get("sender") or ""
            fh.write(f"## {subject}\n\n")
            if date or sender:
                fh.write(f"*{date}* — {sender}\n\n" if sender else f"*{date}*\n\n")
            fh.write(body.rstrip() + "\n")
            if idx < len(items) - 1:
                fh.write("\n---\n\n")

    print(f"Écrit : {out_path} ({len(items)} sources)")


if __name__ == "__main__":
    main()
