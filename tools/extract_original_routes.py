#!/usr/bin/env python3
"""Extract Spring @RequestMapping routes from the original Scala lorsource tree.

The script is intentionally dependency-free. It understands the Spring-style
annotations used by the Scala codebase, including multiline annotations and
regex path variables like {section:(?:forum)|(?:news)}.
"""
from __future__ import annotations

import argparse
import csv
import json
import re
from pathlib import Path
from typing import Iterable


def capture_parens(text: str, open_pos: int) -> tuple[str, int]:
    depth = 0
    quote = False
    esc = False
    for i in range(open_pos, len(text)):
        c = text[i]
        if quote:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                quote = False
        else:
            if c == '"':
                quote = True
            elif c == '(':
                depth += 1
            elif c == ')':
                depth -= 1
                if depth == 0:
                    return text[open_pos + 1 : i], i + 1
    return text[open_pos + 1 :], len(text)


def annotation_at(text: str, start: int) -> str:
    par = text.find('(', start)
    if par == -1:
        return text[start : text.find('\n', start)]
    _body, end = capture_parens(text, par)
    return text[start:end]


def array_for_key(annotation: str, key: str) -> str | None:
    m = re.search(rf"\b{re.escape(key)}\s*=\s*Array\s*\(", annotation)
    if not m:
        return None
    open_pos = annotation.find('(', m.start())
    body, _ = capture_parens(annotation, open_pos)
    return body


def default_array(annotation: str) -> str | None:
    m = re.search(r"@RequestMapping\s*\(\s*Array\s*\(", annotation)
    if not m:
        return None
    open_pos = annotation.find('(', annotation.find('Array', m.start()))
    body, _ = capture_parens(annotation, open_pos)
    return body


def string_values(body: str | None) -> list[str]:
    if body is None:
        return []
    return re.findall(r'"((?:\\.|[^"\\])*)"', body)


def methods_values(body: str | None) -> list[str]:
    if body is None:
        return []
    vals = re.findall(r"RequestMethod\.([A-Z]+)", body)
    vals += [v for v in string_values(body) if v.isupper()]
    return list(dict.fromkeys(vals))


def annotation_paths(annotation: str) -> list[str]:
    return (
        string_values(array_for_key(annotation, "path"))
        or string_values(array_for_key(annotation, "value"))
        or string_values(default_array(annotation))
    )


def annotation_methods(annotation: str) -> list[str]:
    return methods_values(array_for_key(annotation, "method")) or ["ANY"]


def annotation_params(annotation: str) -> list[str]:
    return string_values(array_for_key(annotation, "params"))


def annotation_produces(annotation: str) -> list[str]:
    return string_values(array_for_key(annotation, "produces"))


def clean(s: str) -> str:
    return re.sub(r"\s+", " ", s.strip())


def normalize_path(path: str) -> str:
    # Preserve path variable names, drop Spring regex constraints.
    path = path or ""
    path = re.sub(r"\{([A-Za-z_][A-Za-z0-9_]*):\\{1,2}d\{\d+\}\}", r"{\1}", path)
    path = re.sub(r"\{([A-Za-z_][A-Za-z0-9_]*):\\{1,2}d\+\}", r"{\1}", path)
    path = re.sub(r"\{([^}:]+):[^}]+\}", r"{\1}", path)
    # Spring allows relative class-level paths; Axum route declarations are absolute.
    if path and not path.startswith("/"):
        path = "/" + path
    return path


def join_paths(base: str, sub: str) -> str:
    base = base or ""
    sub = sub or ""
    if not base and not sub:
        return "/"
    if not base:
        return sub
    if not sub:
        return base
    return base.rstrip("/") + "/" + sub.lstrip("/")


def extract_routes(source_root: Path) -> list[dict[str, object]]:
    scala_root = source_root / "src/main/scala"
    rows: list[dict[str, object]] = []

    for file in sorted(scala_root.rglob("*.scala")):
        text = file.read_text(errors="ignore")
        class_annotations: list[tuple[int, str, list[str]]] = []

        for cm in re.finditer(r"(?:class|object)\s+(\w+)", text):
            prefix = text[max(0, cm.start() - 1200) : cm.start()]
            idx = prefix.rfind("@RequestMapping")
            if idx != -1:
                ann = annotation_at(prefix, idx)
                after = prefix[idx + len(ann) :]
                if re.fullmatch(r"[\s@\w\.\(\),]*", after):
                    class_annotations.append((cm.end(), cm.group(1), annotation_paths(ann) or [""]))

        for am in re.finditer(r"@RequestMapping\s*\(", text):
            ann = annotation_at(text, am.start())
            tail = text[am.start() + len(ann) : am.start() + len(ann) + 800]
            dm = re.search(r"(?:\s*@[^\n]+)*\s*def\s+(\w+)", tail)
            cls = re.search(r"(?:\s*@[^\n]+)*\s*(?:class|object)\s+(\w+)", tail)
            if cls and (not dm or cls.start() < dm.start()):
                continue
            if not dm:
                continue

            controller = file.stem
            base_paths = [""]
            for end, name, paths in class_annotations:
                if end < am.start():
                    controller = name
                    base_paths = paths

            previous_classes = list(re.finditer(r"(?:class|object)\s+(\w+)", text[: am.start()]))
            if previous_classes:
                controller = previous_classes[-1].group(1)

            local_paths = annotation_paths(ann) or [""]
            line = text.count("\n", 0, am.start()) + 1
            for base in base_paths:
                for local in local_paths:
                    rows.append(
                        {
                            "controller": controller,
                            "handler": dm.group(1),
                            "source": str(file.relative_to(source_root)),
                            "line": line,
                            "path": normalize_path(join_paths(base, local)),
                            "methods": annotation_methods(ann),
                            "params": annotation_params(ann),
                            "produces": annotation_produces(ann),
                            "annotation": clean(ann),
                        }
                    )

    seen = set()
    out = []
    for row in rows:
        key = (
            row["controller"],
            row["handler"],
            row["path"],
            tuple(row["methods"]),
            tuple(row["params"]),
            row["line"],
        )
        if key not in seen:
            seen.add(key)
            out.append(row)
    out.sort(key=lambda r: (str(r["path"]), str(r["controller"]), int(r["line"])))
    return out


def write_csv(rows: Iterable[dict[str, object]], output: Path) -> None:
    with output.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(
            fh,
            fieldnames=["methods", "path", "params", "produces", "controller", "handler", "source", "line", "annotation"],
        )
        writer.writeheader()
        for row in rows:
            writer.writerow(
                {
                    **row,
                    "methods": ",".join(row["methods"]),
                    "params": ",".join(row["params"]),
                    "produces": ",".join(row["produces"]),
                }
            )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source_root", type=Path, help="path to original Scala lorsource root")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--csv", type=Path)
    args = parser.parse_args()
    routes = extract_routes(args.source_root)
    if args.json:
        args.json.write_text(json.dumps(routes, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.csv:
        write_csv(routes, args.csv)
    if not args.json and not args.csv:
        print(json.dumps(routes, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
