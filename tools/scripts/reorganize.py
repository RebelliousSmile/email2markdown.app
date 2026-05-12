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
import json
import os
import shutil

from src.config import load_config
from src.folder_classifier import _load_corpus


def _build_tree(output_dir: Path, max_depth: int = 3) -> list[tuple[int, int, Path, str]]:
    nodes: list[tuple[int, int, Path, str]] = []

    def _walk(path: Path, depth: int) -> None:
        if depth > max_depth:
            return
        for child in sorted(path.iterdir()):
            if child.is_dir():
                nodes.append((len(nodes) + 1, depth, child, child.name))
                _walk(child, depth + 1)

    _walk(output_dir, 1)
    return nodes


def _print_tree(nodes: list[tuple[int, int, Path, str]]) -> None:
    print()
    for num, depth, _path, name in nodes:
        indent = "  " * (depth - 1)
        print(f"{num:3}. {indent}{name}")
    print()


def _get_node(nodes: list[tuple[int, int, Path, str]], prompt: str) -> tuple[int, int, Path, str] | None:
    raw = input(prompt).strip()
    if not raw.isdigit():
        print("Numéro invalide.")
        return None
    idx = int(raw)
    matches = [n for n in nodes if n[0] == idx]
    if not matches:
        print(f"Nœud {idx} introuvable.")
        return None
    return matches[0]


def _unique_dest_path(dest: Path) -> Path:
    if not dest.exists():
        return dest
    counter = 1
    while True:
        candidate = dest.parent / f"{dest.name}-{counter}"
        if not candidate.exists():
            return candidate
        counter += 1


def _save_corpus(corpus_path: Path, entries: list[dict]) -> None:
    with corpus_path.open("w", encoding="utf-8") as fh:
        for entry in entries:
            fh.write(json.dumps(entry, ensure_ascii=False) + "\n")


def _update_corpus(corpus_path: Path, old_abs: Path, new_abs: Path, output_dir: Path) -> int:
    entries = _load_corpus(corpus_path.parent)
    try:
        old_label = str(old_abs.relative_to(output_dir)).replace("\\", "/")
        new_label = str(new_abs.relative_to(output_dir)).replace("\\", "/")
    except ValueError:
        # Path not inside output_dir — nothing to update
        return 0
    updated = 0
    for entry in entries:
        label = entry.get("label", "")
        if old_label in label:
            entry["label"] = label.replace(old_label, new_label)
            updated += 1
        path_val = entry.get("path", "")
        if path_val and old_label in path_val:
            entry["path"] = path_val.replace(old_label, new_label)
            updated += 1
    if updated:
        _save_corpus(corpus_path, entries)
        print(f"{updated} entrées corpus mises à jour.")
    return updated


def _op_rename(nodes: list[tuple[int, int, Path, str]], corpus_path: Path, output_dir: Path) -> None:
    node = _get_node(nodes, "Numéro du nœud à renommer : ")
    if node is None:
        return
    _, _, node_path, old_name = node
    new_name = input(f"Nouveau nom pour '{old_name}' : ").strip()
    if not new_name:
        print("Nom vide, annulé.")
        return
    confirm = input(f"Renommer '{old_name}' → '{new_name}' ? [o/n] ").strip().lower()
    if confirm != "o":
        print("Annulé.")
        return
    new_path = node_path.parent / new_name
    os.rename(node_path, new_path)
    _update_corpus(corpus_path, node_path, new_path, output_dir)


def _op_merge(nodes: list[tuple[int, int, Path, str]], corpus_path: Path, output_dir: Path) -> None:
    src_node = _get_node(nodes, "Numéro du nœud source : ")
    if src_node is None:
        return
    dst_node = _get_node(nodes, "Numéro du nœud cible (même niveau) : ")
    if dst_node is None:
        return

    _, src_depth, src_path, src_name = src_node
    _, dst_depth, dst_path, dst_name = dst_node

    if src_depth != dst_depth:
        print(f"Les nœuds doivent être au même niveau (source: {src_depth}, cible: {dst_depth}).")
        return
    if src_path == dst_path:
        print("Source et cible identiques, annulé.")
        return

    confirm = input(f"Fusionner '{src_name}' dans '{dst_name}' ? [o/n] ").strip().lower()
    if confirm != "o":
        print("Annulé.")
        return

    for src_file in sorted(src_path.rglob("*.md")):
        rel = src_file.relative_to(src_path)
        dest = _unique_dest_path(dst_path / rel)
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.move(str(src_file), str(dest))
        _update_corpus(corpus_path, src_file, dest, output_dir)

    shutil.rmtree(src_path)
    print(f"'{src_name}' fusionné dans '{dst_name}'.")


def _op_move(nodes: list[tuple[int, int, Path, str]], corpus_path: Path, output_dir: Path) -> None:
    src_node = _get_node(nodes, "Numéro du nœud à déplacer : ")
    if src_node is None:
        return
    parent_node = _get_node(nodes, "Numéro du nouveau parent : ")
    if parent_node is None:
        return

    _, _, src_path, src_name = src_node
    _, _, parent_path, parent_name = parent_node

    if src_path == parent_path or parent_path.is_relative_to(src_path):
        print("Déplacement impossible : cible dans le sous-arbre source.")
        return

    confirm = input(f"Déplacer '{src_name}' sous '{parent_name}' ? [o/n] ").strip().lower()
    if confirm != "o":
        print("Annulé.")
        return

    new_path = _unique_dest_path(parent_path / src_name)
    shutil.move(str(src_path), str(new_path))
    _update_corpus(corpus_path, src_path, new_path, output_dir)
    print(f"'{src_name}' déplacé sous '{parent_name}'.")


def main() -> None:
    parser = argparse.ArgumentParser(description="Restructure interactivement l'arborescence de destination des emails classés.")
    parser.add_argument("--config", default="config/config.yaml", help="Chemin du fichier de configuration")
    args = parser.parse_args()

    config = load_config(Path(args.config))
    classify_cfg = config.get("classify", {})
    output_dir = Path(classify_cfg.get("output_dir", "classified"))
    data_dir = Path(classify_cfg.get("data_dir", "data"))
    corpus_path = data_dir / "corpus.jsonl"

    if not output_dir.exists():
        print(f"Répertoire de destination introuvable : {output_dir}", file=sys.stderr)
        sys.exit(1)

    while True:
        nodes = _build_tree(output_dir)
        if not nodes:
            print("Aucun sous-dossier trouvé dans le répertoire de destination.")
            break
        _print_tree(nodes)
        print("Actions : [r]enommer  [f]usionner  [d]éplacer  [q]uitter")
        choice = input("> ").strip().lower()

        if choice == "q":
            break
        elif choice == "r":
            _op_rename(nodes, corpus_path, output_dir)
        elif choice == "f":
            _op_merge(nodes, corpus_path, output_dir)
        elif choice == "d":
            _op_move(nodes, corpus_path, output_dir)
        else:
            print("Choix invalide.")


if __name__ == "__main__":
    main()
