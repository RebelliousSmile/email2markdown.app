import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent.parent))

# Ensure UTF-8 output on Windows
if sys.stdout.encoding and sys.stdout.encoding.lower() != "utf-8":
    import io
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
if sys.stderr.encoding and sys.stderr.encoding.lower() != "utf-8":
    import io
    sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", errors="replace")

import argparse
import logging
import shutil

from src.config import load_config
from src.parser import parse_email
from src.categorizer import categorize
from src.deduplicator import deduplicate
from src.grouper import group_emails
from src.archiver import archive
from src.summarizers import travail, notification, newsletter, associatif
from src.summarizers.filename import make_filename
from src.folder_classifier import propose_path, record_decision, prompt_user

logging.basicConfig(level=logging.WARNING, format="%(levelname)s: %(message)s")
logger = logging.getLogger(__name__)


def _get_summarizer(category: str):
    mapping = {
        "travail": travail,
        "notification": notification,
        "newsletter": newsletter,
        "associatif": associatif,
    }
    return mapping.get(category, travail)


def _classify_output(
    output_path: Path,
    category: str,
    group_emails_list: list,
    config: dict,
    no_classify: bool,
) -> bool:
    """Move the output file to the classify tree. Returns False if the user quit."""
    if no_classify:
        return True
    classify_output = Path(config.get("classify", {}).get("output_dir", ""))
    if not classify_output or not str(classify_output).strip():
        return True
    meta = {
        "subject": group_emails_list[0].get("subject", ""),
        "sender": group_emails_list[0].get("sender", ""),
        "email_type": category,
    }
    proposed = propose_path(meta, config)
    response = prompt_user(meta, proposed)
    if response == "q":
        print("Classification interrompue par l'utilisateur.")
        return False
    if response == "s":
        return True
    final = proposed if response == "" else response
    parts = [p.strip() for p in final.split("/")]
    if len(parts) == 3:
        dest = classify_output / parts[0] / parts[1] / parts[2] / output_path.name
        if dest.exists():
            stem, suffix, ctr = dest.stem, dest.suffix, 1
            while dest.exists():
                dest = dest.parent / f"{stem}-{ctr}{suffix}"
                ctr += 1
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.move(str(output_path), str(dest))
        record_decision(meta, final, config)
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Summarize emails from a folder.")
    parser.add_argument("--input", default=None, help="Input folder path (overrides paths.input_dir from config)")
    parser.add_argument("--input-files", nargs="*", default=None, help="Explicit list of .md files (overrides --input)")
    parser.add_argument("--input-files-stdin", action="store_true", help="Read list of .md files from stdin (one path per line)")
    parser.add_argument("--notes-dir", default=None, help="Notes output folder (overrides paths.notes_dir from config)")
    parser.add_argument("--output", default=None, help="[deprecated] alias for --notes-dir")
    parser.add_argument(
        "--delete", action="store_true", help="Delete source files instead of archiving"
    )
    parser.add_argument(
        "--config", default="config/config.yaml", help="Config file path"
    )
    parser.add_argument(
        "--no-classify", action="store_true", help="Skip interactive classification step even if classify.output_dir is set"
    )
    args = parser.parse_args()

    config_path = Path(args.config)
    delete_sources = args.delete

    config = load_config(config_path)
    paths = config.get("paths", {})

    file_list: list[str] | None = None
    if args.input_files:
        file_list = list(args.input_files)
    elif args.input_files_stdin:
        file_list = [ln.strip() for ln in sys.stdin if ln.strip()]

    if file_list:
        md_files = sorted(Path(p) for p in file_list if Path(p).exists())
        # input_dir is used for the processed/ fallback — pick the parent of the first file
        input_dir = md_files[0].parent if md_files else Path(".")
    elif args.input:
        input_dir = Path(args.input)
        md_files = None  # will be globbed below
    else:
        input_dir_cfg = paths.get("input_dir")
        if not input_dir_cfg:
            print("Erreur : --input non fourni et paths.input_dir absent de la config.")
            sys.exit(1)
        input_dir = Path(input_dir_cfg)
        md_files = None

    notes_dir_arg = args.notes_dir or args.output
    if notes_dir_arg:
        output_dir = Path(notes_dir_arg)
    else:
        notes_dir = paths.get("notes_dir")
        if not notes_dir:
            print("Erreur : --notes-dir non fourni et paths.notes_dir absent de la config.")
            sys.exit(1)
        output_dir = Path(notes_dir)

    processed_dir_cfg = paths.get("processed_dir")
    default_processed_dir = processed_dir_cfg if processed_dir_cfg else str(input_dir.parent / "processed")

    # Find all .md files in input (non-recursive) unless an explicit list was provided
    if md_files is None:
        md_files = sorted(input_dir.glob("*.md"))
    if not md_files:
        print("Aucun fichier .md trouve dans le dossier d'entree.")
        print("Traite: 0 groupes -> 0 fichiers generes, 0 ignores, 0 erreurs")
        sys.exit(0)

    # Parse emails — skip invalid
    emails = []
    for filepath in md_files:
        try:
            email = parse_email(filepath)
            emails.append(email)
        except (ValueError, Exception) as exc:
            logger.warning("Fichier ignoré %s: %s", filepath.name, exc)

    # Deduplicate
    emails = deduplicate(emails)

    # Categorize
    for email in emails:
        email["category"] = categorize(email)

    # Group
    groups = group_emails(emails)

    # Prepare output directory
    output_dir.mkdir(parents=True, exist_ok=True)

    # Prepare processed directory — from config, or sibling of input as fallback
    processed_dir = Path(default_processed_dir)

    ollama_cfg = config.get("ollama", {})
    ollama_model: str = ollama_cfg.get("model", "qwen3:8b")
    ollama_url: str = ollama_cfg.get("url", "http://localhost:11434")

    generated = 0
    ignored = 0
    errors = []

    for group_id, group_emails_list in groups.items():
        # Sort emails by date ascending
        group_emails_list = sorted(group_emails_list, key=lambda e: e["date"])

        # Determine category from the group_id prefix
        category = group_id.split("::")[0]

        try:
            # Get the appropriate summarizer
            summarizer_module = _get_summarizer(category)

            # Call summarizer
            result = summarizer_module.summarize(group_emails_list, config)

            if result is None:
                # Newsletter with no link — skip and archive
                ignored += 1
                for email in group_emails_list:
                    archive(email["filepath"], processed_dir, delete=delete_sources)
                continue

            # Generate filename
            filename = make_filename(group_emails_list, category, ollama_model, ollama_url)

            # Ensure no collision: append counter if needed
            output_path = output_dir / filename
            if output_path.exists():
                stem = output_path.stem
                suffix = output_path.suffix
                counter = 1
                while output_path.exists():
                    output_path = output_dir / f"{stem}-{counter}{suffix}"
                    counter += 1

            # Write output file
            output_path.write_text(result, encoding="utf-8")
            generated += 1

            user_quit = not _classify_output(output_path, category, group_emails_list, config, args.no_classify)

            # Archive sources
            for email in group_emails_list:
                archive(email["filepath"], processed_dir, delete=delete_sources)

            if user_quit:
                break

        except Exception as exc:
            errors.append(f"Groupe '{group_id}': {exc}")
            logger.error("Erreur sur le groupe '%s': %s", group_id, exc)

    total_groups = len(groups)
    print(
        f"Traite: {total_groups} groupes -> {generated} fichiers generes, "
        f"{ignored} ignores, {len(errors)} erreurs"
    )

    if errors:
        print("\nErreurs rencontrees:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        sys.exit(1)
    else:
        sys.exit(0)


if __name__ == "__main__":
    main()
