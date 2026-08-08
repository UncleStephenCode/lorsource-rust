#!/usr/bin/env python3
"""Extract the original lorsource HTTP surface without third-party parsers.

The primary output is a Spring MVC declaration inventory.  It deliberately
keeps both a comparison-friendly path (``path``) and the original Spring path
expression (``spring_path``); route-count equality is not a semantic parity
claim.  A second, optional artifact inventories WebSocket registrations,
urlrewrite rules, servlet/resource mappings and the default-servlet static
surface.

This is a conservative source extractor, not a Scala/Java compiler.  Exact
annotation metadata is marked ``high`` confidence.  Literal model/view names
and direct ``getParameter`` calls are useful but explicitly marked heuristic.
"""
from __future__ import annotations

import argparse
import csv
import json
import re
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Iterable, Iterator


_IDENT = r"[A-Za-z_$][A-Za-z0-9_$]*"
_CLASS_RE = re.compile(rf"\b(?:class|object|interface|record)\s+({_IDENT})")


def capture_balanced(text: str, open_pos: int, opener: str = "(", closer: str = ")") -> tuple[str, int]:
    """Return balanced contents and the position after the closing delimiter."""
    depth = 0
    quote: str | None = None
    escaped = False
    line_comment = False
    block_comment = False
    i = open_pos
    while i < len(text):
        c = text[i]
        nxt = text[i + 1] if i + 1 < len(text) else ""
        if line_comment:
            if c == "\n":
                line_comment = False
        elif block_comment:
            if c == "*" and nxt == "/":
                block_comment = False
                i += 1
        elif quote:
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == quote:
                quote = None
        elif c in {'"', "'"}:
            quote = c
        elif c == "/" and nxt == "/":
            line_comment = True
            i += 1
        elif c == "/" and nxt == "*":
            block_comment = True
            i += 1
        elif c == opener:
            depth += 1
        elif c == closer:
            depth -= 1
            if depth == 0:
                return text[open_pos + 1 : i], i + 1
        i += 1
    return text[open_pos + 1 :], len(text)


def capture_parens(text: str, open_pos: int) -> tuple[str, int]:
    """Backward-compatible name used by older callers/tests."""
    return capture_balanced(text, open_pos)


def code_mask(text: str) -> str:
    """Blank comments and string contents while retaining offsets/newlines."""
    chars = list(text)
    quote: str | None = None
    escaped = False
    line_comment = False
    block_comment = False
    i = 0
    while i < len(chars):
        c = chars[i]
        nxt = chars[i + 1] if i + 1 < len(chars) else ""
        if line_comment:
            if c == "\n":
                line_comment = False
            else:
                chars[i] = " "
        elif block_comment:
            if c == "*" and nxt == "/":
                chars[i] = chars[i + 1] = " "
                block_comment = False
                i += 1
            elif c != "\n":
                chars[i] = " "
        elif quote:
            if escaped:
                escaped = False
                if c != "\n":
                    chars[i] = " "
            elif c == "\\":
                escaped = True
                chars[i] = " "
            elif c == quote:
                chars[i] = " "
                quote = None
            elif c != "\n":
                chars[i] = " "
        elif c == "/" and nxt == "/":
            chars[i] = chars[i + 1] = " "
            line_comment = True
            i += 1
        elif c == "/" and nxt == "*":
            chars[i] = chars[i + 1] = " "
            block_comment = True
            i += 1
        elif c in {'"', "'"}:
            chars[i] = " "
            quote = c
        i += 1
    return "".join(chars)


def _annotation_extent(text: str, start: int) -> int:
    match = re.match(rf"@(?:{_IDENT})(?:\.(?:{_IDENT}))*", text[start:])
    if not match:
        return start
    end = start + match.end()
    cursor = end
    while cursor < len(text) and text[cursor].isspace():
        cursor += 1
    if cursor < len(text) and text[cursor] == "(":
        _body, end = capture_parens(text, cursor)
    elif cursor < len(text) and text[cursor] == "[":
        _body, end = capture_balanced(text, cursor, "[", "]")
    return end


def annotation_at(text: str, start: int) -> str:
    """Capture an annotation, including a bare ``@RequestMapping``."""
    return text[start : _annotation_extent(text, start)]


def _annotation_body(annotation: str) -> str:
    open_pos = annotation.find("(")
    if open_pos == -1:
        return ""
    body, _end = capture_parens(annotation, open_pos)
    return body


def _named_expression(annotation: str, key: str) -> str | None:
    body = _annotation_body(annotation)
    match = re.search(rf"\b{re.escape(key)}\s*=", body)
    if not match:
        return None
    start = match.end()
    while start < len(body) and body[start].isspace():
        start += 1
    depth = 0
    quote: str | None = None
    escaped = False
    for i in range(start, len(body)):
        c = body[i]
        if quote:
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == quote:
                quote = None
        elif c in {'"', "'"}:
            quote = c
        elif c in "([{":
            depth += 1
        elif c in ")]}" and depth:
            depth -= 1
        elif c == "," and depth == 0:
            return body[start:i].strip()
    return body[start:].strip()


def array_for_key(annotation: str, key: str) -> str | None:
    """Backward-compatible helper; now accepts Scala and Java array syntax."""
    return _named_expression(annotation, key)


def default_array(annotation: str) -> str | None:
    body = _annotation_body(annotation).strip()
    if not body or re.match(rf"(?:path|value|method|params|headers|consumes|produces)\s*=", body):
        return None
    depth = 0
    quote: str | None = None
    escaped = False
    for i, c in enumerate(body):
        if quote:
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == quote:
                quote = None
        elif c in {'"', "'"}:
            quote = c
        elif c in "([{":
            depth += 1
        elif c in ")]}" and depth:
            depth -= 1
        elif c == "," and depth == 0:
            return body[:i].strip()
    return body


def string_values(body: str | None) -> list[str]:
    if body is None:
        return []
    return [value.replace(r'\"', '"') for value in re.findall(r'"((?:\\.|[^"\\])*)"', body)]


def methods_values(body: str | None) -> list[str]:
    if body is None:
        return []
    values = re.findall(r"RequestMethod\.([A-Z]+)", body)
    values += [value for value in string_values(body) if value.isupper()]
    return list(dict.fromkeys(values))


def annotation_paths(annotation: str) -> list[str]:
    return (
        string_values(_named_expression(annotation, "path"))
        or string_values(_named_expression(annotation, "value"))
        or string_values(default_array(annotation))
    )


def annotation_methods(annotation: str, default_any: bool = True) -> list[str]:
    values = methods_values(_named_expression(annotation, "method"))
    return values or (["ANY"] if default_any else [])


def _annotation_strings(annotation: str, key: str) -> list[str]:
    return string_values(_named_expression(annotation, key))


def annotation_params(annotation: str) -> list[str]:
    return _annotation_strings(annotation, "params")


def annotation_headers(annotation: str) -> list[str]:
    return _annotation_strings(annotation, "headers")


def annotation_consumes(annotation: str) -> list[str]:
    return _annotation_strings(annotation, "consumes")


def annotation_produces(annotation: str) -> list[str]:
    return _annotation_strings(annotation, "produces")


def clean(value: str) -> str:
    return re.sub(r"\s+", " ", value.strip())


def normalize_path(path: str) -> str:
    """Drop Spring regexes without losing nested regex quantifier braces."""
    path = path or ""
    output: list[str] = []
    i = 0
    while i < len(path):
        if path[i] != "{":
            output.append(path[i])
            i += 1
            continue
        name_match = re.match(r"\{([A-Za-z_][A-Za-z0-9_]*)", path[i:])
        if not name_match:
            output.append(path[i])
            i += 1
            continue
        name = name_match.group(1)
        cursor = i + name_match.end()
        if cursor < len(path) and path[cursor] == "}":
            output.append(path[i : cursor + 1])
            i = cursor + 1
            continue
        if cursor >= len(path) or path[cursor] != ":":
            output.append(path[i])
            i += 1
            continue
        depth = 1
        cursor += 1
        while cursor < len(path) and depth:
            if path[cursor] == "{":
                depth += 1
            elif path[cursor] == "}":
                depth -= 1
            cursor += 1
        output.append("{" + name + "}")
        i = cursor
    normalized = "".join(output)
    if normalized and not normalized.startswith("/"):
        normalized = "/" + normalized
    return normalized


def join_paths(base: str, sub: str) -> str:
    base = base or ""
    sub = sub or ""
    if not base and not sub:
        return "/"
    if not base:
        return sub if sub.startswith("/") else "/" + sub
    if not sub:
        return base if base.startswith("/") else "/" + base
    result = base.rstrip("/") + "/" + sub.lstrip("/")
    return result if result.startswith("/") else "/" + result


def _skip_trivia(text: str, pos: int) -> int:
    while pos < len(text):
        if text[pos].isspace():
            pos += 1
        elif text.startswith("//", pos):
            newline = text.find("\n", pos + 2)
            pos = len(text) if newline == -1 else newline + 1
        elif text.startswith("/*", pos):
            end = text.find("*/", pos + 2)
            pos = len(text) if end == -1 else end + 2
        else:
            break
    return pos


def _target_after_mapping(text: str, annotation_end: int) -> dict[str, object] | None:
    pos = _skip_trivia(text, annotation_end)
    following_annotations: list[str] = []
    while pos < len(text) and text[pos] == "@":
        end = _annotation_extent(text, pos)
        if end == pos:
            break
        following_annotations.append(text[pos:end])
        pos = _skip_trivia(text, end)

    class_match = re.match(
        rf"(?:(?:public|private|protected|final|abstract|sealed|case|static)\s+)*"
        rf"(?:class|object|interface|record)\s+({_IDENT})",
        text[pos:],
    )
    if class_match:
        declaration_keyword = re.search(r"\b(?:class|object|interface|record)\b", class_match.group(0))
        return {
            "kind": "class",
            "name": class_match.group(1),
            "decl_pos": pos + (declaration_keyword.start() if declaration_keyword else class_match.start()),
            "annotations": following_annotations,
        }

    scala_match = re.match(
        rf"(?:(?:override|private|protected|final|implicit|lazy|inline)\s+)*def\s+({_IDENT})",
        text[pos:],
    )
    if scala_match:
        name = scala_match.group(1)
        name_end = pos + scala_match.end()
        params_open = text.find("(", name_end, name_end + 120)
        params = ""
        signature_end = name_end
        if params_open != -1:
            between = text[name_end:params_open]
            if not re.search(r"[:={\n]", between):
                params, signature_end = capture_parens(text, params_open)
        return {
            "kind": "handler",
            "language": "scala",
            "name": name,
            "decl_pos": pos,
            "name_pos": pos + scala_match.start(1),
            "params": params,
            "signature_end": signature_end,
            "annotations": following_annotations,
        }

    # Java controller methods in this tree use ordinary visibility/modifier syntax.
    brace = text.find("{", pos, pos + 2000)
    semicolon = text.find(";", pos, pos + 2000)
    limit_candidates = [value for value in (brace, semicolon) if value != -1]
    limit = min(limit_candidates) if limit_candidates else min(len(text), pos + 2000)
    declaration = text[pos:limit]
    method_match = re.search(rf"({_IDENT})\s*\(", declaration)
    if method_match:
        name = method_match.group(1)
        params_open = pos + method_match.end() - 1
        params, signature_end = capture_parens(text, params_open)
        return {
            "kind": "handler",
            "language": "java",
            "name": name,
            "decl_pos": pos,
            "name_pos": pos + method_match.start(1),
            "params": params,
            "signature_end": signature_end,
            "annotations": following_annotations,
        }
    return None


def _combine_methods(class_methods: list[str], handler_methods: list[str]) -> list[str]:
    if not class_methods:
        return handler_methods or ["ANY"]
    if not handler_methods:
        return class_methods
    return [method for method in handler_methods if method in class_methods]


def _combine_media(class_values: list[str], handler_values: list[str]) -> list[str]:
    # Spring method declarations override/narrow class-level media conditions.
    return handler_values or class_values


def _split_parameters(params: str) -> list[str]:
    rows: list[str] = []
    start = 0
    depth = 0
    quote: str | None = None
    escaped = False
    for i, c in enumerate(params):
        if quote:
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == quote:
                quote = None
        elif c in {'"', "'"}:
            quote = c
        elif c in "([{":
            depth += 1
        elif c in ")]}" and depth:
            depth -= 1
        elif c == "," and depth == 0:
            rows.append(params[start:i].strip())
            start = i + 1
    tail = params[start:].strip()
    if tail:
        rows.append(tail)
    return rows


def _annotations_in(fragment: str) -> list[tuple[str, str, int, int]]:
    rows = []
    for match in re.finditer(rf"@({_IDENT})(?:\.(?:{_IDENT}))*", fragment):
        end = _annotation_extent(fragment, match.start())
        rows.append((match.group(1), fragment[match.start():end], match.start(), end))
    return rows


def _blank_annotations(fragment: str) -> str:
    chars = list(fragment)
    for _name, _annotation, start, end in _annotations_in(fragment):
        chars[start:end] = " " * (end - start)
    return "".join(chars)


def _scalar_annotation_value(annotation: str, keys: tuple[str, ...] = ("value", "name")) -> str | None:
    for key in keys:
        values = string_values(_named_expression(annotation, key))
        if values:
            return values[0]
    values = string_values(default_array(annotation))
    return values[0] if values else None


def _parameter_name_and_type(fragment: str, language: str) -> tuple[str | None, str | None]:
    plain = clean(_blank_annotations(fragment))
    if language == "scala":
        match = re.search(rf"\b({_IDENT})\s*:\s*(.+?)(?:\s*=.+)?$", plain)
        return (match.group(1), clean(match.group(2))) if match else (None, None)
    match = re.search(rf"(.+?)\s+({_IDENT})(?:\s*=.+)?$", plain)
    return (match.group(2), clean(match.group(1))) if match else (None, None)


def _parameter_metadata(params: str, language: str, form_fields: dict[str, list[str]]) -> tuple[list[dict], list[dict], list[dict]]:
    request_params: list[dict] = []
    path_variables: list[dict] = []
    model_attributes: list[dict] = []
    for fragment in _split_parameters(params):
        annotations = _annotations_in(fragment)
        variable, type_name = _parameter_name_and_type(fragment, language)
        for name, annotation, _start, _end in annotations:
            if name not in {"RequestParam", "PathVariable", "ModelAttribute"}:
                continue
            external = _scalar_annotation_value(annotation) or variable
            required_expr = _named_expression(annotation, "required")
            required = None
            if name in {"RequestParam", "PathVariable"}:
                required = not (required_expr and required_expr.strip().lower() == "false")
            default_expr = _named_expression(annotation, "defaultValue")
            default_values = string_values(default_expr)
            default = default_values[0] if default_values else None
            if default is not None and name == "RequestParam":
                required = False
            item = {
                "name": external,
                "parameter": variable,
                "type": type_name,
                "required": required,
                "default": default,
                "source": "annotation",
                "confidence": "high",
            }
            if name == "RequestParam":
                request_params.append(item)
            elif name == "PathVariable":
                path_variables.append(item)
            else:
                model_type = (type_name or "").replace(" ", "")
                fields = form_fields.get(model_type, form_fields.get(model_type.split(".")[-1], []))
                model_attributes.append(
                    {
                        "name": external,
                        "parameter": variable,
                        "type": type_name,
                        "fields": fields,
                        "source": "annotation+bean-accessors",
                        "confidence": "high" if fields else "medium",
                    }
                )
    return request_params, path_variables, model_attributes


def _find_closing_brace(mask: str, open_pos: int) -> int:
    depth = 0
    for i in range(open_pos, len(mask)):
        if mask[i] == "{":
            depth += 1
        elif mask[i] == "}":
            depth -= 1
            if depth == 0:
                return i + 1
    return len(mask)


def _handler_region(text: str, mask: str, target: dict[str, object], fallback_end: int) -> str:
    start = int(target["decl_pos"])
    signature_end = int(target["signature_end"])
    if target["language"] == "java":
        brace = mask.find("{", signature_end, min(fallback_end, signature_end + 1000))
        if brace != -1:
            return text[start:_find_closing_brace(mask, brace)]
    else:
        equals = mask.find("=", signature_end, min(fallback_end, signature_end + 1500))
        if equals != -1:
            brace = mask.find("{", equals, min(fallback_end, equals + 800))
            if brace != -1:
                return text[start:_find_closing_brace(mask, brace)]

    line_start = text.rfind("\n", 0, start) + 1
    indent = len(text[line_start:start]) - len(text[line_start:start].lstrip(" \t"))
    boundary = re.compile(
        rf"(?m)^[ \t]{{0,{indent}}}(?:@(?:RequestMapping|ExceptionHandler|ModelAttribute|InitBinder)\b|"
        rf"(?:(?:private|protected|public|override|final|static)\s+)*def\s+{_IDENT}\b)"
    ).search(mask, max(signature_end, text.find("\n", start) + 1), fallback_end)
    return text[start : boundary.start() if boundary else fallback_end]


def extract_bindable_fields(source_root: Path) -> dict[str, list[str]]:
    """Find JavaBean setters and Scala ``@BeanProperty`` form fields."""
    values: dict[str, set[str]] = {}
    source_dirs = [source_root / "src/main/java", source_root / "src/main/scala"]
    for source_dir in source_dirs:
        if not source_dir.exists():
            continue
        for file in sorted(source_dir.rglob("*.java")):
            text = file.read_text(encoding="utf-8", errors="ignore")
            mask = code_mask(text)
            blocks: list[dict[str, object]] = []
            for match in re.finditer(rf"\bclass\s+({_IDENT})", mask):
                brace = mask.find("{", match.end())
                if brace == -1:
                    continue
                blocks.append({"name": match.group(1), "start": match.start(), "end": _find_closing_brace(mask, brace)})
            for setter in re.finditer(rf"\bset([A-Z][A-Za-z0-9_$]*)\s*\(", mask):
                containers = [block for block in blocks if int(block["start"]) < setter.start() < int(block["end"])]
                if not containers:
                    continue
                block = min(containers, key=lambda item: int(item["end"]) - int(item["start"]))
                parents = [
                    parent for parent in blocks
                    if int(parent["start"]) < int(block["start"]) and int(block["end"]) < int(parent["end"])
                ]
                qualified = ".".join([str(item["name"]) for item in sorted(parents, key=lambda item: int(item["start"]))] + [str(block["name"])])
                field = setter.group(1)[0].lower() + setter.group(1)[1:]
                values.setdefault(qualified, set()).add(field)
                values.setdefault(str(block["name"]), set()).add(field)

        for file in sorted(source_dir.rglob("*.scala")):
            text = file.read_text(encoding="utf-8", errors="ignore")
            mask = code_mask(text)
            classes = list(re.finditer(rf"\bclass\s+({_IDENT})", mask))
            for index, match in enumerate(classes):
                end = classes[index + 1].start() if index + 1 < len(classes) else len(text)
                fragment = text[match.end():end]
                fields = re.findall(
                    rf"@(?:BeanProperty|BooleanBeanProperty)\b(?:\s*\([^)]*\))?\s*"
                    rf"(?:private(?:\[[^]]+\])?\s+)?var\s+({_IDENT})",
                    fragment,
                )
                if fields:
                    values.setdefault(match.group(1), set()).update(fields)
    return {name: sorted(fields) for name, fields in sorted(values.items())}


def extract_view_content_types(source_root: Path) -> tuple[dict[str, str], list[str]]:
    jsp_types: dict[str, str] = {}
    jsp_root = source_root / "src/main/webapp/WEB-INF/jsp"
    if jsp_root.exists():
        for file in sorted(jsp_root.rglob("*.jsp")):
            text = file.read_text(encoding="utf-8", errors="ignore")
            match = re.search(r"<%@\s*page\b[^%]*?\bcontentType\s*=\s*\"([^\"]+)\"", text, re.S)
            if match:
                jsp_types[str(file.relative_to(jsp_root).with_suffix(""))] = match.group(1)

    configured_feed_types: list[str] = []
    servlet_xml = source_root / "src/main/webapp/WEB-INF/springapp-servlet.xml"
    root = _xml_root(servlet_xml)
    if root is not None:
        for property_element in root.iter():
            if _tag(property_element) != "property" or property_element.attrib.get("name") != "contentTypes":
                continue
            configured_feed_types.extend(
                child.attrib["value"] for child in property_element.iter()
                if _tag(child) == "entry" and child.attrib.get("value")
            )
    return jsp_types, list(dict.fromkeys(configured_feed_types))


def _nearby_handler_annotations(text: str, mapping_start: int, following: list[str]) -> list[str]:
    names = [match.group(1) for annotation in following for match in [re.match(rf"@({_IDENT})", annotation)] if match]
    prefix_start = max(0, mapping_start - 1000)
    prefix = text[prefix_start:mapping_start]
    for wanted in ("ResponseBody", "ResponseStatus", "CSRFNoAuto"):
        pos = prefix.rfind("@" + wanted)
        if pos == -1:
            continue
        between = code_mask(prefix[pos + len(wanted) + 1 :])
        if not re.search(r"[{};=]|\b(?:def|class|object)\b", between):
            names.append(wanted)
    return list(dict.fromkeys(names))


def _response_status(text: str, mapping_start: int, following: list[str]) -> str | None:
    for candidate in following:
        match = re.search(r"@ResponseStatus\s*\([^)]*?(?:HttpStatus\.)?([A-Z][A-Z0-9_]*)", candidate)
        if match:
            return match.group(1)
    prefix_start = max(0, mapping_start - 1000)
    prefix = text[prefix_start:mapping_start]
    status_pos = prefix.rfind("@ResponseStatus")
    if status_pos != -1:
        status_end = _annotation_extent(prefix, status_pos)
        between = code_mask(prefix[status_end:])
        if not re.search(r"[{};=]|\b(?:def|class|object)\b", between):
            candidate = prefix[status_pos:status_end]
            match = re.search(r"@ResponseStatus\s*\([^)]*?(?:HttpStatus\.)?([A-Z][A-Z0-9_]*)", candidate)
            if match:
                return match.group(1)
    return None


def _return_type(text: str, target: dict[str, object]) -> str | None:
    name_pos = int(target["name_pos"])
    signature_end = int(target["signature_end"])
    if target["language"] == "scala":
        fragment = text[signature_end : min(len(text), signature_end + 500)]
        match = re.search(r":\s*([^=\n]+?)\s*=", fragment)
        return clean(match.group(1)) if match else None
    prefix = clean(text[int(target["decl_pos"]):name_pos])
    tokens = prefix.split()
    return tokens[-1] if tokens else None


def _response_kind(response_body: bool, return_type: str | None, produces: list[str], region: str) -> str:
    lower_produces = " ".join(produces).lower()
    if response_body and (return_type and re.search(r"(?:^|\[)Json(?:\]|$)", return_type)):
        return "json"
    if response_body or produces:
        if "json" in lower_produces:
            return "json"
        if "xml" in lower_produces or "rss" in lower_produces or "atom" in lower_produces:
            return "feed"
        return "response-body"
    if '"feed-type"' in region or re.search(r'new\s+ModelAndView\s*\(\s*"[^\"]*rss[^\"]*"', region, re.I):
        return "feed"
    if return_type and "RedirectView" in return_type:
        return "redirect"
    if "new RedirectView" in region:
        return "redirect-or-view"
    if return_type and "ModelAndView" in return_type:
        return "html-view"
    return "unknown"


def _condition(annotation: str) -> dict[str, list[str]]:
    return {
        "methods": annotation_methods(annotation, default_any=False),
        "params": annotation_params(annotation),
        "headers": annotation_headers(annotation),
        "consumes": annotation_consumes(annotation),
        "produces": annotation_produces(annotation),
    }


def _source_files(source_root: Path) -> Iterator[Path]:
    for relative, suffix in (("src/main/scala", "*.scala"), ("src/main/java", "*.java")):
        root = source_root / relative
        if root.exists():
            yield from sorted(root.rglob(suffix))


def extract_routes(source_root: Path) -> list[dict[str, object]]:
    form_fields = extract_bindable_fields(source_root)
    jsp_content_types, configured_feed_types = extract_view_content_types(source_root)
    rows: list[dict[str, object]] = []

    for file in _source_files(source_root):
        text = file.read_text(encoding="utf-8", errors="ignore")
        mask = code_mask(text)
        occurrences = list(re.finditer(r"@RequestMapping\b", mask))
        mappings: list[dict[str, object]] = []
        for occurrence in occurrences:
            annotation = annotation_at(text, occurrence.start())
            annotation_end = occurrence.start() + len(annotation)
            target = _target_after_mapping(text, annotation_end)
            if target:
                mappings.append(
                    {
                        "start": occurrence.start(),
                        "annotation": annotation,
                        "target": target,
                    }
                )

        class_mappings: dict[int, list[dict[str, object]]] = {}
        for mapping in mappings:
            target = mapping["target"]
            if target["kind"] == "class":
                class_mappings.setdefault(int(target["decl_pos"]), []).append(mapping)

        classes = [(match.start(), match.group(1)) for match in _CLASS_RE.finditer(mask)]
        controller_model_attributes: dict[int, list[dict[str, object]]] = {}
        for attribute_match in re.finditer(r"@ModelAttribute\b", mask):
            attribute_annotation = annotation_at(text, attribute_match.start())
            provider = _target_after_mapping(text, attribute_match.start() + len(attribute_annotation))
            if not provider or provider["kind"] != "handler" or provider.get("language") != "scala":
                continue
            provider_classes = [item for item in classes if item[0] < attribute_match.start()]
            if not provider_classes:
                continue
            provider_class_pos, _provider_controller = provider_classes[-1]
            controller_model_attributes.setdefault(provider_class_pos, []).append(
                {
                    "name": _scalar_annotation_value(attribute_annotation),
                    "provider": provider["name"],
                    "return_type": _return_type(text, provider),
                    "source": "controller-model-provider",
                    "confidence": "high",
                }
            )
        controller_exception_handlers: dict[int, list[dict[str, object]]] = {}
        for exception_match in re.finditer(r"@ExceptionHandler\b", mask):
            exception_annotation = annotation_at(text, exception_match.start())
            exception_target = _target_after_mapping(text, exception_match.start() + len(exception_annotation))
            if not exception_target or exception_target["kind"] != "handler":
                continue
            exception_classes = [item for item in classes if item[0] < exception_match.start()]
            if not exception_classes:
                continue
            exception_class_pos, _exception_controller = exception_classes[-1]
            exception_types = re.findall(r"classOf\[([^]]+)]|([A-Za-z_$][A-Za-z0-9_$.]*)\.class", exception_annotation)
            flattened_types = [left or right for left, right in exception_types]
            controller_exception_handlers.setdefault(exception_class_pos, []).append(
                {
                    "handler": exception_target["name"],
                    "exceptions": flattened_types,
                    "response_status": _response_status(
                        text, exception_match.start(), list(exception_target["annotations"])
                    ),
                    "return_type": _return_type(text, exception_target),
                    "source": "controller-exception-handler",
                    "confidence": "high",
                }
            )
        handler_mappings = [mapping for mapping in mappings if mapping["target"]["kind"] == "handler"]
        for mapping_index, mapping in enumerate(handler_mappings):
            target = mapping["target"]
            mapping_start = int(mapping["start"])
            enclosing = [item for item in classes if item[0] < mapping_start]
            class_pos, controller = enclosing[-1] if enclosing else (0, file.stem)
            class_variants = class_mappings.get(class_pos, [])
            if not class_variants:
                class_variants = [{"annotation": "", "start": class_pos}]

            handler_annotation = str(mapping["annotation"])
            handler_condition = _condition(handler_annotation)
            handler_paths = annotation_paths(handler_annotation) or [""]
            next_start = (
                int(handler_mappings[mapping_index + 1]["start"])
                if mapping_index + 1 < len(handler_mappings)
                else len(text)
            )
            region = _handler_region(text, mask, target, next_start)
            annotations = _nearby_handler_annotations(text, mapping_start, list(target["annotations"]))
            response_body = "ResponseBody" in annotations
            return_type = _return_type(text, target)
            request_params, path_variables, model_attributes = _parameter_metadata(
                str(target["params"]), str(target["language"]), form_fields
            )
            known_params = {item["name"] for item in request_params}
            for direct_name in re.findall(r"\.getParameter(?:Values)?\s*\(\s*\"([^\"]+)\"", region):
                if direct_name not in known_params:
                    request_params.append(
                        {
                            "name": direct_name,
                            "parameter": None,
                            "type": None,
                            "required": None,
                            "default": None,
                            "source": "direct-getParameter",
                            "confidence": "medium",
                        }
                    )
                    known_params.add(direct_name)
            model_keys = sorted(set(re.findall(r"\.addObject\s*\(\s*\"([^\"]+)\"", region)))
            view_names = sorted(
                set(re.findall(r"new\s+ModelAndView\s*\(\s*\"([^\"]+)\"", region))
                | set(re.findall(r"\.setViewName\s*\(\s*\"([^\"]+)\"", region))
            )
            response_status = _response_status(text, mapping_start, list(target["annotations"]))

            for class_mapping in class_variants:
                class_annotation = str(class_mapping["annotation"])
                class_condition = _condition(class_annotation) if class_annotation else {
                    "methods": [], "params": [], "headers": [], "consumes": [], "produces": []
                }
                base_paths = annotation_paths(class_annotation) or [""]
                methods = _combine_methods(class_condition["methods"], handler_condition["methods"])
                params = class_condition["params"] + handler_condition["params"]
                headers = class_condition["headers"] + handler_condition["headers"]
                consumes = _combine_media(class_condition["consumes"], handler_condition["consumes"])
                produces = _combine_media(class_condition["produces"], handler_condition["produces"])
                response_content_types = [
                    {"value": value, "source": "RequestMapping.produces", "confidence": "high"}
                    for value in produces
                ]
                for view_name in view_names:
                    if view_name in jsp_content_types:
                        response_content_types.append(
                            {
                                "value": jsp_content_types[view_name],
                                "source": f"WEB-INF/jsp/{view_name}.jsp page directive",
                                "confidence": "high",
                            }
                        )
                if response_body and return_type and re.search(r"(?:^|\[)Json(?:\]|$)", return_type) and not produces:
                    response_content_types.append(
                        {
                            "value": "application/json",
                            "source": "@ResponseBody Json + configured message converter",
                            "confidence": "medium",
                        }
                    )
                if '"feed-type"' in region:
                    response_content_types.extend(
                        {
                            "value": value,
                            "source": "AbstractRomeView contentTypes configuration",
                            "confidence": "medium",
                        }
                        for value in configured_feed_types
                        if value not in {item["value"] for item in response_content_types}
                    )
                for base_path in base_paths:
                    for handler_path in handler_paths:
                        spring_path = join_paths(base_path, handler_path)
                        form_field_names = sorted(
                            {field for attribute in model_attributes for field in attribute.get("fields", [])}
                        )
                        rows.append(
                            {
                                "controller": controller,
                                "handler": target["name"],
                                "source": str(file.relative_to(source_root)),
                                "source_language": target["language"],
                                "line": text.count("\n", 0, mapping_start) + 1,
                                "path": normalize_path(spring_path),
                                "spring_path": spring_path,
                                "class_path": base_path,
                                "method_path": handler_path,
                                "path_has_constraints": normalize_path(spring_path) != spring_path,
                                "methods": methods or ["NONE"],
                                "declared_methods": handler_condition["methods"] or ["ANY"],
                                "class_methods": class_condition["methods"] or ["ANY"],
                                "params": params,
                                "headers": headers,
                                "consumes": consumes,
                                "produces": produces,
                                "declared_conditions": handler_condition,
                                "class_conditions": class_condition,
                                "request_params": request_params,
                                "path_variables": path_variables,
                                "model_attributes": model_attributes,
                                "controller_model_attributes": controller_model_attributes.get(class_pos, []),
                                "controller_exception_handlers": controller_exception_handlers.get(class_pos, []),
                                "form_fields": form_field_names,
                                "model_keys": model_keys,
                                "view_names": view_names,
                                "handler_annotations": annotations,
                                "response_body": response_body,
                                "response_kind": _response_kind(response_body, return_type, produces, region),
                                "response_content_types": response_content_types,
                                "response_status": response_status,
                                "return_type": return_type,
                                "mapping_is_bare": "(" not in handler_annotation,
                                "annotation": clean(handler_annotation),
                                "class_annotation": clean(class_annotation) if class_annotation else None,
                                "metadata_confidence": {
                                    "mapping": "high",
                                    "annotated_parameters": "high",
                                    "direct_parameters": "medium",
                                    "literal_model_and_view": "medium",
                                },
                            }
                        )

    seen: set[tuple[object, ...]] = set()
    output = []
    for row in rows:
        key = (
            row["controller"], row["handler"], row["spring_path"], tuple(row["methods"]),
            tuple(row["params"]), tuple(row["headers"]), row["line"],
        )
        if key not in seen:
            seen.add(key)
            output.append(row)
    output.sort(key=lambda row: (str(row["path"]), str(row["spring_path"]), str(row["controller"]), int(row["line"])))
    return output


def _xml_root(path: Path) -> ET.Element | None:
    if not path.exists():
        return None
    text = path.read_text(encoding="utf-8", errors="ignore")
    text = re.sub(r"<!DOCTYPE[\s\S]*?>", "", text, count=1)
    try:
        return ET.fromstring(text)
    except ET.ParseError:
        return None


def _tag(element: ET.Element) -> str:
    return element.tag.rsplit("}", 1)[-1]


def extract_original_surface(source_root: Path) -> dict[str, object]:
    websocket: list[dict[str, object]] = []
    resource_handlers: list[dict[str, object]] = []
    controller_advice: list[dict[str, object]] = []
    for file in _source_files(source_root):
        text = file.read_text(encoding="utf-8", errors="ignore")
        if re.search(r"@ControllerAdvice\b", code_mask(text)):
            class_match = _CLASS_RE.search(code_mask(text))
            order_match = re.search(r"@Order\s*\(([^)]*)\)", text)
            disallowed_fields: list[str] = []
            for call in re.finditer(r"setDisallowedFields\s*\(([^)]*)\)", text):
                argument = clean(call.group(1))
                assignment = re.search(
                    rf"(?:String\s*\[\]|val|var)\s+{re.escape(argument)}\s*=\s*(?:new\s+String\s*\[\]\s*)?\{{?([^;]+)",
                    text,
                )
                if assignment:
                    disallowed_fields.extend(string_values(assignment.group(1)))
            controller_advice.append(
                {
                    "kind": "controller_advice",
                    "class": class_match.group(1) if class_match else file.stem,
                    "order_expression": clean(order_match.group(1)) if order_match else None,
                    "disallowed_binding_fields": list(dict.fromkeys(disallowed_fields)),
                    "source": str(file.relative_to(source_root)),
                }
            )
        for match in re.finditer(r"addHandler\s*\([^,]+,\s*([^)]*)\)", text):
            for path in string_values(match.group(1)):
                tail = text[match.end():match.end() + 300]
                origins = None
                origin_match = re.search(r"setAllowedOrigins\s*\(([^)]*)\)", tail)
                if origin_match:
                    origins = clean(origin_match.group(1))
                protocol_literals = sorted(
                    set(re.findall(r'(?:s)?\"((?:comment\s+\$[^\"]+)|events-refresh)\"', text))
                )
                inbound_contract = None
                if re.search(r"(?:request|payload)\.split\s*\(\s*\" \"\s*,\s*2\s*\)", text):
                    inbound_contract = "text: <topicId> or <topicId> <lastSeenCommentId>"
                websocket.append(
                    {
                        "kind": "websocket",
                        "path": path,
                        "handshake_method": "GET (Upgrade: websocket)",
                        "allowed_origins_expression": origins,
                        "inbound_payload_contract": inbound_contract,
                        "outbound_message_literals": protocol_literals,
                        "authentication_principal": (
                            "RememberMeAuthenticationToken/UserDetailsImpl"
                            if "RememberMeAuthenticationToken" in text and "UserDetailsImpl" in text else None
                        ),
                        "closes_with_server_error_on_handler_exception": "CloseStatus.SERVER_ERROR" in text,
                        "source": str(file.relative_to(source_root)),
                        "line": text.count("\n", 0, match.start()) + 1,
                    }
                )
        for match in re.finditer(r"addResourceHandler\s*\(([^)]*)\)", text):
            mappings = string_values(match.group(1))
            tail = text[match.end():match.end() + 500]
            locations_match = re.search(r"addResourceLocations\s*\(([^)]*)\)", tail)
            locations = string_values(locations_match.group(1)) if locations_match else []
            if locations_match and not locations:
                locations = [clean(locations_match.group(1))]
            for path in mappings:
                resource_handlers.append(
                    {
                        "kind": "resource_handler",
                        "path": path,
                        "locations": locations,
                        "source": str(file.relative_to(source_root)),
                        "line": text.count("\n", 0, match.start()) + 1,
                    }
                )

    servlet_xml = source_root / "src/main/webapp/WEB-INF/springapp-servlet.xml"
    servlet_root = _xml_root(servlet_xml)
    default_servlet = False
    interceptors: list[dict[str, object]] = []
    if servlet_root is not None:
        for element in servlet_root.iter():
            if _tag(element) == "default-servlet-handler":
                default_servlet = True
            elif _tag(element) == "resources":
                resource_handlers.append(
                    {
                        "kind": "resource_handler",
                        "path": element.attrib.get("mapping", ""),
                        "locations": [element.attrib.get("location", "")],
                        "cache_period": element.attrib.get("cache-period"),
                        "source": str(servlet_xml.relative_to(source_root)),
                        "line": None,
                    }
                )
            elif _tag(element) == "interceptors":
                for child in element:
                    if _tag(child) == "bean" and child.attrib.get("class"):
                        interceptors.append(
                            {
                                "kind": "spring_mvc_interceptor",
                                "class": child.attrib["class"],
                                "path": "/** (unless excluded by nested config)",
                                "source": str(servlet_xml.relative_to(source_root)),
                            }
                        )

    rewrite_xml = source_root / "src/main/webapp/WEB-INF/urlrewrite.xml"
    rewrite_root = _xml_root(rewrite_xml)
    rewrite_rules: list[dict[str, object]] = []
    if rewrite_root is not None:
        for element in list(rewrite_root):
            kind = _tag(element)
            if kind not in {"rule", "outbound-rule"}:
                continue
            from_element = next((child for child in element if _tag(child) == "from"), None)
            to_element = next((child for child in element if _tag(child) == "to"), None)
            conditions = [
                {"attributes": dict(child.attrib), "value": (child.text or "").strip()}
                for child in element if _tag(child) == "condition"
            ]
            actions = [
                {"tag": _tag(child), "attributes": dict(child.attrib), "value": (child.text or "").strip()}
                for child in element if _tag(child) == "set"
            ]
            rewrite_rules.append(
                {
                    "kind": kind.replace("-", "_"),
                    "from": (from_element.text or "").strip() if from_element is not None else None,
                    "to": (to_element.text or "").strip() if to_element is not None else None,
                    "to_attributes": dict(to_element.attrib) if to_element is not None else {},
                    "conditions": conditions,
                    "actions": actions,
                    "source": str(rewrite_xml.relative_to(source_root)),
                }
            )

    web_xml = source_root / "src/main/webapp/WEB-INF/web.xml"
    web_root = _xml_root(web_xml)
    servlet_mappings: list[dict[str, object]] = []
    filter_mappings: list[dict[str, object]] = []
    error_pages: list[dict[str, object]] = []
    webapp_settings: dict[str, object] = {"default_servlet_handler": default_servlet}
    if web_root is not None:
        for element in web_root.iter():
            element_tag = _tag(element)
            if element_tag == "servlet-mapping":
                servlet_name = next((child.text for child in element if _tag(child) == "servlet-name"), None)
                patterns = [(child.text or "").strip() for child in element if _tag(child) == "url-pattern"]
                for pattern in patterns:
                    servlet_mappings.append(
                        {
                            "kind": "servlet_mapping",
                            "path": pattern,
                            "servlet": (servlet_name or "").strip(),
                            "source": str(web_xml.relative_to(source_root)),
                        }
                    )
            elif element_tag == "filter-mapping":
                filter_name = next((child.text for child in element if _tag(child) == "filter-name"), None)
                patterns = [(child.text or "").strip() for child in element if _tag(child) == "url-pattern"]
                dispatchers = [(child.text or "").strip() for child in element if _tag(child) == "dispatcher"]
                for pattern in patterns:
                    filter_mappings.append(
                        {
                            "kind": "filter_mapping",
                            "path": pattern,
                            "filter": (filter_name or "").strip(),
                            "dispatchers": dispatchers,
                            "source": str(web_xml.relative_to(source_root)),
                        }
                    )
            elif element_tag == "error-page":
                code = next(((child.text or "").strip() for child in element if _tag(child) == "error-code"), None)
                exception = next(((child.text or "").strip() for child in element if _tag(child) == "exception-type"), None)
                location = next(((child.text or "").strip() for child in element if _tag(child) == "location"), None)
                error_pages.append(
                    {
                        "kind": "error_page",
                        "path": location,
                        "error_code": code,
                        "exception_type": exception,
                        "source": str(web_xml.relative_to(source_root)),
                    }
                )
            elif element_tag == "session-timeout":
                webapp_settings["session_timeout_minutes"] = (element.text or "").strip()
            elif element_tag == "multipart-config":
                webapp_settings["multipart"] = {
                    _tag(child).replace("-", "_"): (child.text or "").strip() for child in element
                }

    static_roots: list[dict[str, object]] = []
    webapp = source_root / "src/main/webapp"
    if webapp.exists():
        for child in sorted(webapp.iterdir()):
            if child.name in {"WEB-INF", "META-INF"}:
                continue
            if child.is_dir():
                files = [path for path in child.rglob("*") if path.is_file()]
                static_roots.append(
                    {
                        "kind": "static_root",
                        "path": f"/{child.name}/**",
                        "file_count": len(files),
                        "extensions": sorted({path.suffix.lower() or "(none)" for path in files}),
                        "served_by": "default servlet" if default_servlet else "container-dependent",
                        "source": str(child.relative_to(source_root)),
                    }
                )
            elif child.is_file():
                static_roots.append(
                    {
                        "kind": "static_file",
                        "path": "/" + child.name,
                        "file_count": 1,
                        "extensions": [child.suffix.lower() or "(none)"],
                        "served_by": "default servlet" if default_servlet else "container-dependent",
                        "source": str(child.relative_to(source_root)),
                    }
                )

    return {
        "websocket": websocket,
        "url_rewrite": rewrite_rules,
        "resource_handlers": resource_handlers,
        "servlet_mappings": servlet_mappings,
        "filter_mappings": filter_mappings,
        "error_pages": error_pages,
        "interceptors": interceptors,
        "controller_advice": controller_advice,
        "static_surface": static_roots,
        "webapp_settings": webapp_settings,
        "notes": [
            "This artifact inventories declarations and literal static roots; it does not prove runtime reachability.",
            "URL rewrite/filter ordering can change the externally observed behavior of an MVC mapping.",
            "WebSocket message parsing and side effects still require behavioral tests.",
        ],
    }


_ROUTE_CSV_FIELDS = [
    "methods", "path", "spring_path", "params", "headers", "consumes", "produces",
    "controller", "handler", "source_language", "response_body", "response_kind", "response_status",
    "response_content_types",
    "request_params", "path_variables", "model_attributes", "controller_model_attributes",
    "controller_exception_handlers",
    "form_fields", "model_keys", "view_names",
    "source", "line", "annotation", "class_annotation",
]


def write_csv(rows: Iterable[dict[str, object]], output: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=_ROUTE_CSV_FIELDS, extrasaction="ignore")
        writer.writeheader()
        for row in rows:
            rendered = dict(row)
            for key in ("methods", "params", "headers", "consumes", "produces", "form_fields", "model_keys", "view_names"):
                rendered[key] = ",".join(str(value) for value in row[key])
            for key in (
                "request_params", "path_variables", "model_attributes", "controller_model_attributes",
                "controller_exception_handlers", "response_content_types",
            ):
                rendered[key] = json.dumps(row[key], ensure_ascii=False, separators=(",", ":"))
            writer.writerow(rendered)


def write_surface_csv(surface: dict[str, object], output: Path) -> None:
    rows: list[dict[str, str]] = []
    for section in (
        "websocket", "url_rewrite", "resource_handlers", "servlet_mappings", "filter_mappings",
        "error_pages", "interceptors", "static_surface",
        "controller_advice",
    ):
        for item in surface[section]:
            item_dict = dict(item)
            path = item_dict.get("path") or item_dict.get("from") or ""
            source = item_dict.get("source") or ""
            rows.append(
                {
                    "surface": section,
                    "kind": str(item_dict.get("kind", "")),
                    "path_or_pattern": str(path),
                    "source": str(source),
                    "details_json": json.dumps(item_dict, ensure_ascii=False, sort_keys=True, separators=(",", ":")),
                }
            )
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=["surface", "kind", "path_or_pattern", "source", "details_json"])
        writer.writeheader()
        writer.writerows(rows)


def write_summary_md(routes: list[dict[str, object]], surface: dict[str, object], output: Path) -> None:
    handlers = {(row["source"], row["line"], row["handler"]) for row in routes}
    model_providers = {
        (row["controller"], provider["provider"], provider["name"])
        for row in routes for provider in row["controller_model_attributes"]
    }
    exception_handlers = {
        (row["controller"], handler["handler"])
        for row in routes for handler in row["controller_exception_handlers"]
    }
    controllers: dict[str, list[dict[str, object]]] = {}
    for row in routes:
        controllers.setdefault(str(row["controller"]), []).append(row)
    method_counts: dict[str, int] = {}
    for row in routes:
        label = "+".join(row["methods"])
        method_counts[label] = method_counts.get(label, 0) + 1
    lines = [
        "# Current original controller and HTTP-surface inventory",
        "",
        "> Generated from Java/Scala declarations. These counts are inventory data, not evidence of semantic parity with the Rust port.",
        "",
        f"- Spring handler methods: **{len(handlers)}**",
        f"- Expanded Spring mapping variants: **{len(routes)}**",
        f"- Unique normalized MVC path templates: **{len({row['path'] for row in routes})}**",
        f"- Controllers with mapped handlers: **{len(controllers)}**",
        f"- `@ResponseBody` mapping variants: **{sum(bool(row['response_body']) for row in routes)}**",
        f"- Bare method-level `@RequestMapping` variants: **{sum(bool(row['mapping_is_bare']) for row in routes)}**",
        f"- Mapping variants with Spring regex path constraints: **{sum(bool(row['path_has_constraints']) for row in routes)}**",
        f"- Controller-wide `@ModelAttribute` providers: **{len(model_providers)}**",
        f"- Controller `@ExceptionHandler` methods: **{len(exception_handlers)}**",
        "",
        "Declared effective methods: " + ", ".join(f"`{key}` {value}" for key, value in sorted(method_counts.items())),
        "",
        "Non-MVC surface:",
        "",
        f"- WebSocket registrations: **{len(surface['websocket'])}**",
        f"- URL rewrite/filter rules: **{len(surface['url_rewrite'])}**",
        f"- Spring resource handler patterns: **{len(surface['resource_handlers'])}**",
        f"- servlet URL mappings: **{len(surface['servlet_mappings'])}**",
        f"- servlet filter mappings: **{len(surface['filter_mappings'])}**",
        f"- servlet error-page dispatches: **{len(surface['error_pages'])}**",
        f"- Spring MVC interceptors: **{len(surface['interceptors'])}**",
        f"- global controller-advice declarations: **{len(surface['controller_advice'])}**",
        f"- default-servlet static roots/files: **{len(surface['static_surface'])}**",
        "",
        "The detailed machine-readable contracts are in `docs/generated/current_java_routes.json` and `docs/generated/current_java_surface.json`.",
        "",
        "| Controller | Handler methods | Expanded variants | Unique paths | Sources |",
        "|---|---:|---:|---:|---|",
    ]
    for controller, controller_rows in sorted(controllers.items()):
        controller_handlers = {(row["source"], row["line"], row["handler"]) for row in controller_rows}
        sources = sorted({str(row["source"]) for row in controller_rows})
        lines.append(
            f"| `{controller}` | {len(controller_handlers)} | {len(controller_rows)} | "
            f"{len({row['path'] for row in controller_rows})} | `{'`, `'.join(sources)}` |"
        )
    lines += [
        "",
        "## Interpretation limits",
        "",
        "The extractor records declared mapping conditions (`params`, `headers`, `consumes`, `produces`), annotated parameters, bean-bindable form fields, `@ResponseBody`, literal view/model keys and direct `getParameter` calls. It does not execute Spring, follow the full service call graph, resolve security configuration, prove template equivalence or observe database/external-system side effects. Runtime differential tests remain required.",
    ]
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source_root", type=Path, help="path to the original Java/Scala lorsource root")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--csv", type=Path)
    parser.add_argument("--surface-json", type=Path)
    parser.add_argument("--surface-csv", type=Path)
    parser.add_argument("--summary-md", type=Path)
    args = parser.parse_args()
    routes = extract_routes(args.source_root)
    if args.json:
        _write_json(args.json, routes)
    if args.csv:
        write_csv(routes, args.csv)
    if args.surface_json or args.surface_csv or args.summary_md:
        surface = extract_original_surface(args.source_root)
        if args.surface_json:
            _write_json(args.surface_json, surface)
        if args.surface_csv:
            write_surface_csv(surface, args.surface_csv)
        if args.summary_md:
            write_summary_md(routes, surface, args.summary_md)
    if not any((args.json, args.csv, args.surface_json, args.surface_csv, args.summary_md)):
        print(json.dumps(routes, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
