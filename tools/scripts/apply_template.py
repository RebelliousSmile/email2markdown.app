"""Render a Jinja2 template against a list of email-markdown files.

Usage:
    apply_template.py --template <path> --input-files <list> [--output <path>]
    apply_template.py --template <path> --input-files-stdin [--output <path>]

Each input file's YAML frontmatter is parsed into a dict; the rendered template
receives:

  * ``emails`` — list of dicts (frontmatter + ``body``/``filename``/``path``),
    sorted by date ascending.
  * ``start_date`` / ``end_date`` — first and last non-empty ``date`` in the corpus.
  * ``count`` — len(emails).
  * ``senders`` — sorted unique list of ``from`` values.
  * ``recipients`` — sorted unique union of every ``to`` and ``cc`` address.
  * ``subjects`` — list of ``subject`` strings in date order.
  * ``now`` — current local datetime ISO string at render time.

If --output is provided, the rendered text is written there; otherwise it is
printed to stdout. ``--input-files-stdin`` reads one path per line from stdin
(used by the Rust caller when the batch is too large for argv).
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
from datetime import datetime

import frontmatter
from jinja2 import Environment, FileSystemLoader, StrictUndefined, select_autoescape

logging.basicConfig(level=logging.WARNING, format="%(levelname)s: %(message)s")
logger = logging.getLogger(__name__)


def _load_email(filepath: Path) -> dict:
    """Parse frontmatter + body, return a flat dict consumable by Jinja templates.

    Pre-populates frequently-referenced fields (``cc``, ``to``, ``sender``,
    ``subject``, ``date``, ``email_type``) so templates running under
    ``StrictUndefined`` can read them via attribute access without try/except.
    """
    post = frontmatter.load(filepath)
    meta = dict(post.metadata or {})
    meta["body"] = post.content
    meta["filename"] = filepath.name
    meta["path"] = str(filepath)
    # Normalise common variants for templates.
    if "from" in meta and "sender" not in meta:
        meta["sender"] = meta["from"]
    for list_key in ("to", "cc"):
        val = meta.get(list_key)
        if val is None:
            meta[list_key] = []
        elif not isinstance(val, list):
            meta[list_key] = [val] if val else []
    # Default any expected scalar to empty string so attribute access never raises.
    for scalar_key in ("subject", "sender", "from", "date", "email_type"):
        meta.setdefault(scalar_key, "")
    return meta


def _date_key(email: dict) -> str:
    d = email.get("date", "")
    return str(d) if d is not None else ""


def _unique_sorted(items):
    """Deduplicate while keeping deterministic ordering — drop empty values."""
    seen = set()
    out = []
    for it in items:
        if not it:
            continue
        if it in seen:
            continue
        seen.add(it)
        out.append(it)
    out.sort()
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description="Render a Jinja2 template against email notes.")
    parser.add_argument("--template", required=True, help="Path to the .md Jinja2 template")
    parser.add_argument(
        "--input-files",
        nargs="+",
        default=None,
        help="Input .md files (mutually exclusive with --input-files-stdin)",
    )
    parser.add_argument(
        "--input-files-stdin",
        action="store_true",
        help="Read one input path per line on stdin instead of --input-files",
    )
    parser.add_argument("--output", default=None, help="Output file path (default: stdout)")
    args = parser.parse_args()

    template_path = Path(args.template)
    if not template_path.exists():
        print(f"Erreur : template introuvable : {template_path}", file=sys.stderr)
        sys.exit(2)

    if args.input_files_stdin:
        raw_inputs = [ln.strip() for ln in sys.stdin if ln.strip()]
    elif args.input_files:
        raw_inputs = list(args.input_files)
    else:
        print("Erreur : aucun fichier fourni (--input-files ou --input-files-stdin)", file=sys.stderr)
        sys.exit(2)

    emails = []
    for raw in raw_inputs:
        fp = Path(raw)
        if not fp.exists():
            logger.warning("Fichier ignoré (absent) : %s", fp)
            continue
        try:
            emails.append(_load_email(fp))
        except Exception as exc:
            logger.warning("Parse échoué %s : %s", fp.name, exc)

    if not emails:
        print("Aucun email valide à rendre.", file=sys.stderr)
        sys.exit(1)

    emails.sort(key=_date_key)
    start_date = _date_key(emails[0])
    end_date = _date_key(emails[-1])

    senders = _unique_sorted(str(e.get("from") or e.get("sender") or "").strip() for e in emails)
    recipients_raw = []
    for e in emails:
        for key in ("to", "cc"):
            val = e.get(key)
            if isinstance(val, list):
                recipients_raw.extend(str(v).strip() for v in val)
            elif val:
                recipients_raw.append(str(val).strip())
    recipients = _unique_sorted(recipients_raw)
    subjects = [str(e.get("subject") or "").strip() for e in emails]

    env = Environment(
        loader=FileSystemLoader(template_path.parent),
        autoescape=select_autoescape(disabled_extensions=("md",), default=False),
        undefined=StrictUndefined,
        trim_blocks=True,
        lstrip_blocks=True,
    )
    template = env.get_template(template_path.name)
    rendered = template.render(
        emails=emails,
        start_date=start_date,
        end_date=end_date,
        count=len(emails),
        senders=senders,
        recipients=recipients,
        subjects=subjects,
        now=datetime.now().isoformat(timespec="seconds"),
    )

    if args.output:
        out_path = Path(args.output)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(rendered, encoding="utf-8")
        print(f"Écrit : {out_path}")
    else:
        sys.stdout.write(rendered)


if __name__ == "__main__":
    main()
