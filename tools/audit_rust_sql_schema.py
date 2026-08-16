#!/usr/bin/env python3
"""Audit SQL embedded in Rust against the canonical Java PostgreSQL schema.

The audit is intentionally conservative.  It reports a missing identifier only
when its owning relation can be resolved (including aliases), or when the
identifier appears in an unambiguous INSERT/UPDATE column list.  Dynamic SQL,
CTEs and table-valued functions are inventoried, but uncertain references are
left for review instead of being promoted to compatibility failures.
"""
from __future__ import annotations

import argparse
import csv
import gzip
import hashlib
import html
import json
import re
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Sequence


SQL_START_RE = re.compile(
    r"^\s*(?:(?:/\*.*?\*/|--[^\n]*(?:\n|$))\s*)*"
    r"(?:SELECT|WITH|INSERT|UPDATE|DELETE|CREATE|ALTER|DROP|TRUNCATE|DO|SET)\b",
    re.IGNORECASE | re.DOTALL,
)

SQL_FRAGMENT_RE = re.compile(
    r"^\s*(?:AND|OR|WHERE|HAVING|CASE|GROUP\s+BY|ORDER\s+BY|(?:LEFT|RIGHT|FULL|INNER|CROSS)\s+JOIN|JOIN)\b",
    re.IGNORECASE,
)

IDENT_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_$]*")

SYSTEM_SCHEMAS = {"information_schema", "pg_catalog"}
SYSTEM_RELATIONS = {
    "databasechangelog",
    "databasechangeloglock",
    "_sqlx_migrations",
}

# PostgreSQL grammar words, built-ins and type names which can occur as bare
# tokens in expressions.  Relation/alias resolution remains the primary column
# check; this set only protects the conservative unqualified-column pass.
NON_COLUMN_WORDS = {
    "all", "analyse", "analyze", "and", "any", "array", "as", "asc",
    "asymmetric", "authorization", "between", "bigint", "binary", "bit",
    "boolean", "both", "by", "case", "cast", "char", "character", "check",
    "coalesce", "collate", "column", "conflict", "constraint", "create", "cross",
    "current_catalog", "current_date", "current_role", "current_schema",
    "current_time", "current_timestamp", "current_user", "date", "day",
    "dec", "decimal", "default", "deferrable", "delete", "desc", "distinct",
    "do", "else", "end", "escape", "except", "exists", "extract", "false", "fetch",
    "filter", "first", "float", "for", "foreign", "from", "full", "grant",
    "greatest", "group", "having", "hour", "ilike", "in", "initially",
    "inner", "insert", "int", "integer", "intersect", "interval", "into",
    "is", "join", "json", "jsonb", "last", "lateral", "leading", "least",
    "left", "like", "limit", "localtime", "localtimestamp", "minute",
    "month", "natural", "new", "no", "not", "nothing", "null", "nullif", "nulls",
    "numeric", "of", "offset", "old", "on", "only", "or", "order", "outer",
    "over", "overlaps", "partition", "placing", "primary", "real", "recursive",
    "references", "returning", "right", "row", "second", "select", "session_user", "share",
    "set", "similar", "smallint", "some", "symmetric", "table", "text", "then",
    "time", "timestamp", "to", "trailing", "true", "union", "unique", "unknown",
    "update", "user", "using", "values", "varchar", "variadic", "verbose", "when",
    "where", "window", "with", "within", "without", "year", "zone",
    # Frequent PostgreSQL functions and pseudo-relations.
    "avg", "bool_and", "bool_or", "count", "date_part", "date_trunc", "decode",
    "encode", "generate_series", "json_agg", "json_build_object", "jsonb_agg",
    "jsonb_build_object", "jsonb_each", "jsonb_object_agg", "lower", "max", "min",
    "now", "pg_get_serial_sequence", "regexp_replace", "row_number", "setval",
    "string_agg", "substring", "sum", "to_char", "to_regclass", "trim", "unnest",
    "upper",
}

BUILTIN_TYPES = {
    "bigint", "bigserial", "bool", "boolean", "bytea", "char", "date", "decimal",
    "double", "float", "hstore", "inet", "int", "int2", "int4", "int8", "integer",
    "interval", "json", "jsonb", "numeric", "real", "serial", "smallint", "text",
    "time", "timestamp", "timestamptz", "uuid", "varchar", "name", "oid", "regclass",
    "regnamespace", "regproc", "regtype",
}


@dataclass(frozen=True)
class RustString:
    source: str
    line: int
    value: str
    raw: bool
    symbol: str | None = None
    test_scope: bool = False


@dataclass(frozen=True)
class Token:
    kind: str
    value: str
    depth: int
    offset: int

    @property
    def lower(self) -> str:
        return self.value.lower()


@dataclass
class Relation:
    table: str | None
    alias: str | None
    schema: str | None
    context: str
    indexes: set[int] = field(default_factory=set)
    virtual: bool = False
    system: bool = False
    end: int = 0


def read_schema_contract(path: Path) -> tuple[dict[str, set[str]], dict[tuple[str, str], str]]:
    tables: dict[str, set[str]] = defaultdict(set)
    types: dict[tuple[str, str], str] = {}
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip() or raw.startswith("#"):
            continue
        fields = raw.split("\t")
        if len(fields) < 3:
            raise ValueError(f"{path}:{number}: expected table, column and PostgreSQL type")
        table, column, pg_type = (value.strip() for value in fields[:3])
        tables[table].add(column)
        types[(table, column)] = pg_type.lstrip("_")
    return dict(tables), types


def _sql_labels(fragment: str) -> set[str]:
    return {
        match.group(1).replace("''", "'")
        for match in re.finditer(r"'((?:''|[^'])*)'", fragment)
    }


def read_java_enums(java_sql_root: Path) -> dict[str, set[str]]:
    """Derive current enum labels from the vendored dump and Liquibase SQL."""
    texts: list[str] = []
    demo = java_sql_root / "demo.db.gz"
    if demo.exists():
        with gzip.open(demo, "rt", encoding="utf-8", errors="replace") as stream:
            texts.append(stream.read())
    for path in sorted(java_sql_root.rglob("*.xml")):
        texts.append(html.unescape(path.read_text(encoding="utf-8", errors="replace")))
    text = "\n".join(texts)
    enums: dict[str, set[str]] = defaultdict(set)
    for match in re.finditer(
        r"\bCREATE\s+TYPE\s+(?:[A-Za-z_][\w$]*\.)?([A-Za-z_][\w$]*)\s+AS\s+ENUM\s*\((.*?)\)",
        text,
        re.IGNORECASE | re.DOTALL,
    ):
        enums[match.group(1).lower()].update(_sql_labels(match.group(2)))
    for match in re.finditer(
        r"\bALTER\s+TYPE\s+(?:[A-Za-z_][\w$]*\.)?([A-Za-z_][\w$]*)\s+"
        r"ADD\s+VALUE(?:\s+IF\s+NOT\s+EXISTS)?\s+'((?:''|[^'])*)'",
        text,
        re.IGNORECASE,
    ):
        enums[match.group(1).lower()].add(match.group(2).replace("''", "'"))
    # PostgreSQL 8/9.0 compatibility migration added enum values directly.
    for match in re.finditer(
        r"INSERT\s+INTO\s+(?:pg_catalog\.)?pg_enum\b.*?'((?:''|[^'])*)'"
        r".*?typname\s*=\s*'_([A-Za-z_][\w$]*)'",
        text,
        re.IGNORECASE | re.DOTALL,
    ):
        enums[match.group(2).lower()].add(match.group(1).replace("''", "'"))
    return {name: values for name, values in sorted(enums.items())}


def _decode_rust_string(value: str) -> str:
    """Decode the common Rust escapes without corrupting non-ASCII source."""
    out: list[str] = []
    i = 0
    escapes = {"n": "\n", "r": "\r", "t": "\t", "0": "\0", '"': '"', "'": "'", "\\": "\\"}
    while i < len(value):
        if value[i] != "\\" or i + 1 >= len(value):
            out.append(value[i])
            i += 1
            continue
        nxt = value[i + 1]
        if nxt in escapes:
            out.append(escapes[nxt])
            i += 2
        elif nxt == "\n":
            i += 2
            while i < len(value) and value[i] in " \t\r\n":
                i += 1
        else:
            # Unicode/hex escapes are irrelevant to SQL identifiers.  Retain
            # them verbatim rather than implementing a subtly different Rust lexer.
            out.extend(("\\", nxt))
            i += 2
    return "".join(out)


def extract_rust_strings(path: Path, root: Path) -> list[RustString]:
    text = path.read_text(encoding="utf-8", errors="replace")
    test_symbols: set[str] = set()
    for match in re.finditer(
        r"(?m)(?P<attrs>(?:^[ \t]*#\s*\[[^\n]*\][ \t]*\n)+)"
        r"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(?P<name>[A-Za-z_][\w]*)",
        text,
    ):
        if re.search(
            r"#\s*\[\s*(?:[A-Za-z_][\w]*::)*test(?:\s*\([^]]*\))?\s*\]",
            match.group("attrs"),
        ):
            test_symbols.add(match.group("name"))
    declarations: list[tuple[int, str]] = []
    for match in re.finditer(
        r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][\w]*)|"
        r"^\s*(?:pub\s+)?const\s+([A-Za-z_][\w]*)",
        text,
    ):
        declarations.append((match.start(), match.group(1) or match.group(2)))

    def symbol_at(offset: int) -> str | None:
        symbol = None
        for position, name in declarations:
            if position > offset:
                break
            symbol = name
        return symbol

    result: list[RustString] = []
    i = 0
    block_depth = 0
    length = len(text)
    while i < length:
        if block_depth:
            if text.startswith("/*", i):
                block_depth += 1
                i += 2
            elif text.startswith("*/", i):
                block_depth -= 1
                i += 2
            else:
                i += 1
            continue
        if text.startswith("//", i):
            newline = text.find("\n", i + 2)
            i = length if newline < 0 else newline + 1
            continue
        if text.startswith("/*", i):
            block_depth = 1
            i += 2
            continue

        # Skip character/byte-character literals.  In particular, a Rust
        # character such as '"' must not be mistaken for the beginning of a
        # string literal. Lifetimes do not match this deliberately narrow form.
        char_match = re.match(r"(?:b)?'(?:\\.|[^'\\])'", text[i:])
        if char_match:
            i += char_match.end()
            continue

        raw_match = re.match(r"(?:b)?r(#{0,255})\"", text[i:])
        if raw_match and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = raw_match.group(1)
            content_start = i + raw_match.end()
            terminator = '"' + hashes
            end = text.find(terminator, content_start)
            if end < 0:
                break
            value = text[content_start:end]
            result.append(
                RustString(
                    str(path.relative_to(root)),
                    text.count("\n", 0, i) + 1,
                    value,
                    True,
                    symbol_at(i),
                    symbol_at(i) in test_symbols,
                )
            )
            i = end + len(terminator)
            continue

        quote_at = i
        if text.startswith('b"', i) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            quote_at = i + 1
        if text[quote_at:quote_at + 1] == '"':
            j = quote_at + 1
            escaped = False
            while j < length:
                char = text[j]
                if char == '"' and not escaped:
                    break
                if char == "\\" and not escaped:
                    escaped = True
                else:
                    escaped = False
                j += 1
            if j >= length:
                break
            value = _decode_rust_string(text[quote_at + 1:j])
            result.append(
                RustString(
                    str(path.relative_to(root)),
                    text.count("\n", 0, i) + 1,
                    value,
                    False,
                    symbol_at(i),
                    symbol_at(i) in test_symbols,
                )
            )
            i = j + 1
            continue
        i += 1
    return result


def looks_like_sql(value: str) -> bool:
    if SQL_START_RE.search(value):
        return True
    if not SQL_FRAGMENT_RE.search(value):
        return False
    # Continuation literals must contain syntax that distinguishes them from
    # human-readable errors beginning with words such as "offset".
    return bool(
        re.search(r"\b[A-Za-z_][\w$]*\s*\.\s*[A-Za-z_][\w$]*\b", value)
        or re.search(r"\$\d+", value)
        or re.search(r"\b(?:GROUP|ORDER)\s+BY\b|\bJOIN\b", value, re.IGNORECASE)
    )


def extract_sql_strings(root: Path) -> list[RustString]:
    strings: list[RustString] = []
    for path in sorted((root / "src").rglob("*.rs")):
        file_strings = extract_rust_strings(path, root)
        selected = [item for item in file_strings if looks_like_sql(item.value)]
        known_aliases: set[str] = set()
        for item in selected:
            if not SQL_START_RE.search(item.value):
                continue
            tokens = tokenize_sql(item.value)
            relations, _ = find_relations(tokens, find_ctes(tokens))
            for relation in relations:
                if relation.table and not relation.virtual and not relation.system:
                    known_aliases.add(relation.table)
                    if relation.alias:
                        known_aliases.add(relation.alias)
        selected_ids = {(item.line, item.value) for item in selected}
        for item in file_strings:
            if (item.line, item.value) in selected_ids:
                continue
            value = item.value.strip()
            # Askama expressions use Rust-style equality.  A template-source
            # assertion such as ``c.answer_count == 1`` can otherwise look
            # like a SQL continuation when the same Rust file contains a real
            # query using alias ``c``.
            if "==" in value:
                continue
            qualifiers = set(re.findall(r"\b([a-z][a-z0-9_]{0,15})\s*\.\s*[a-z_][a-z0-9_$]*\b", value))
            expression_start = re.match(
                r"(?is)^(?:NOT\s+|COALESCE\s*\(|LOWER\s*\(|UPPER\s*\(|[a-z][a-z0-9_]{0,15}\s*\.)",
                value,
            )
            looks_like_markup_or_path = bool(
                re.search(r"<[A-Za-z!/]|(?:^|/)[A-Za-z0-9_{}-]+\.(?:html|jsp|css|jpg|md)\b|https?://", value)
            )
            if qualifiers & known_aliases and expression_start and not looks_like_markup_or_path:
                selected.append(item)
        strings.extend(sorted(selected, key=lambda item: (item.line, item.value)))
    return strings


def tokenize_sql(sql: str) -> list[Token]:
    tokens: list[Token] = []
    i = 0
    depth = 0
    while i < len(sql):
        char = sql[i]
        if char.isspace():
            i += 1
            continue
        if sql.startswith("--", i):
            end = sql.find("\n", i + 2)
            i = len(sql) if end < 0 else end + 1
            continue
        if sql.startswith("/*", i):
            end = sql.find("*/", i + 2)
            i = len(sql) if end < 0 else end + 2
            continue
        dollar = re.match(r"\$([A-Za-z_][A-Za-z0-9_]*)?\$", sql[i:])
        if dollar:
            marker = dollar.group(0)
            end = sql.find(marker, i + len(marker))
            if end < 0:
                end = len(sql) - len(marker)
            tokens.append(Token("string", sql[i + len(marker):end], depth, i))
            i = end + len(marker)
            continue
        if char == "'":
            start = i
            i += 1
            value: list[str] = []
            while i < len(sql):
                if sql[i] == "'" and i + 1 < len(sql) and sql[i + 1] == "'":
                    value.append("'")
                    i += 2
                elif sql[i] == "'":
                    i += 1
                    break
                else:
                    value.append(sql[i])
                    i += 1
            tokens.append(Token("string", "".join(value), depth, start))
            continue
        if char == '"':
            start = i
            i += 1
            value = []
            while i < len(sql):
                if sql[i] == '"' and i + 1 < len(sql) and sql[i + 1] == '"':
                    value.append('"')
                    i += 2
                elif sql[i] == '"':
                    i += 1
                    break
                else:
                    value.append(sql[i])
                    i += 1
            tokens.append(Token("ident", "".join(value), depth, start))
            continue
        if char == "{" and not sql.startswith("{{", i):
            end = sql.find("}", i + 1)
            if end >= 0:
                tokens.append(Token("placeholder", sql[i + 1:end], depth, i))
                i = end + 1
                continue
        parameter = re.match(r"\$\d+", sql[i:])
        if parameter:
            tokens.append(Token("parameter", parameter.group(0), depth, i))
            i += len(parameter.group(0))
            continue
        ident = IDENT_RE.match(sql, i)
        if ident:
            tokens.append(Token("ident", ident.group(0), depth, i))
            i = ident.end()
            continue
        if char == "(":
            tokens.append(Token("punct", char, depth, i))
            depth += 1
            i += 1
            continue
        if char == ")":
            depth = max(depth - 1, 0)
            tokens.append(Token("punct", char, depth, i))
            i += 1
            continue
        matched = False
        for operator in ("::", "->>", "->", ">=", "<=", "<>", "!=", "||", "&&", "@>", "<@"):
            if sql.startswith(operator, i):
                tokens.append(Token("operator", operator, depth, i))
                i += len(operator)
                matched = True
                break
        if matched:
            continue
        tokens.append(Token("punct" if char in ",.;[]" else "operator", char, depth, i))
        i += 1
    return tokens


def _matching_paren(tokens: Sequence[Token], start: int) -> int:
    if start >= len(tokens) or tokens[start].value != "(":
        return start
    depth = tokens[start].depth
    for index in range(start + 1, len(tokens)):
        if tokens[index].value == ")" and tokens[index].depth == depth:
            return index
    return len(tokens) - 1


def find_ctes(tokens: Sequence[Token]) -> set[str]:
    ctes: set[str] = set()
    for start, token in enumerate(tokens):
        if token.kind != "ident" or token.lower != "with":
            continue
        i = start + 1
        if i < len(tokens) and tokens[i].lower == "recursive":
            i += 1
        while i < len(tokens) and tokens[i].kind == "ident":
            name = tokens[i].lower
            i += 1
            if i < len(tokens) and tokens[i].value == "(":
                i = _matching_paren(tokens, i) + 1
            if i >= len(tokens) or tokens[i].lower != "as":
                break
            i += 1
            if i < len(tokens) and tokens[i].lower in {"not", "materialized"}:
                if tokens[i].lower == "not":
                    i += 1
                if i < len(tokens) and tokens[i].lower == "materialized":
                    i += 1
            if i >= len(tokens) or tokens[i].value != "(":
                break
            ctes.add(name)
            i = _matching_paren(tokens, i) + 1
            if i >= len(tokens) or tokens[i].value != ",":
                break
            i += 1
    return ctes


RELATION_END_WORDS = {
    "where", "set", "on", "using", "group", "order", "limit", "offset", "returning",
    "join", "left", "right", "inner", "outer", "full", "cross", "union", "having",
    "values", "conflict", "do", "window", "fetch", "for",
}


def _relation_after(
    tokens: Sequence[Token], start: int, context: str, ctes: set[str]
) -> Relation | None:
    i = start
    while i < len(tokens) and tokens[i].lower in {"only", "lateral"}:
        i += 1
    if i >= len(tokens):
        return None
    indexes: set[int] = set()
    if tokens[i].value == "(":
        end = _matching_paren(tokens, i)
        j = end + 1
        if j < len(tokens) and tokens[j].lower == "as":
            j += 1
        alias = tokens[j].lower if j < len(tokens) and tokens[j].kind == "ident" and tokens[j].lower not in RELATION_END_WORDS else None
        if alias:
            indexes.add(j)
            j += 1
        return Relation(None, alias, None, context, indexes, virtual=True, end=j)
    if tokens[i].kind != "ident":
        return None
    parts = [tokens[i].lower]
    indexes.add(i)
    j = i + 1
    while j + 1 < len(tokens) and tokens[j].value == "." and tokens[j + 1].kind == "ident":
        indexes.update((j, j + 1))
        parts.append(tokens[j + 1].lower)
        j += 2
    if j < len(tokens) and tokens[j].value == "(" and context in {"from", "join", "using"}:
        end = _matching_paren(tokens, j)
        j = end + 1
        if j < len(tokens) and tokens[j].lower == "as":
            j += 1
        alias = tokens[j].lower if j < len(tokens) and tokens[j].kind == "ident" and tokens[j].lower not in RELATION_END_WORDS else None
        if alias:
            indexes.add(j)
            j += 1
        return Relation(None, alias, None, context, indexes, virtual=True, end=j)
    table = parts[-1]
    schema = parts[-2] if len(parts) > 1 else None
    alias = None
    if j < len(tokens) and tokens[j].lower == "as":
        indexes.add(j)
        j += 1
        if j < len(tokens) and tokens[j].kind == "ident":
            alias = tokens[j].lower
            indexes.add(j)
            j += 1
    elif j < len(tokens) and tokens[j].kind == "ident" and tokens[j].lower not in RELATION_END_WORDS:
        alias = tokens[j].lower
        indexes.add(j)
        j += 1
    virtual = table in ctes
    system = schema in SYSTEM_SCHEMAS or table.startswith("pg_") or table in SYSTEM_RELATIONS
    return Relation(table, alias, schema, context, indexes, virtual, system, j)


def _enclosing_function(tokens: Sequence[Token], index: int) -> str | None:
    target_depth = tokens[index].depth
    for i in range(index - 1, -1, -1):
        if tokens[i].value == "(" and tokens[i].depth == target_depth - 1:
            if i and tokens[i - 1].kind == "ident":
                return tokens[i - 1].lower
            return None
    return None


def _is_create_trigger(tokens: Sequence[Token]) -> bool:
    header = [token.lower for token in tokens[:5]]
    return (
        header[:2] == ["create", "trigger"]
        or header[:3] == ["create", "constraint", "trigger"]
        or header[:4] == ["create", "or", "replace", "trigger"]
        or header[:5] == ["create", "or", "replace", "constraint", "trigger"]
    )


def find_relations(tokens: Sequence[Token], ctes: set[str]) -> tuple[list[Relation], set[int]]:
    relations: list[Relation] = []
    relation_indexes: set[int] = set()
    create_trigger = _is_create_trigger(tokens)
    if create_trigger:
        trigger_on = next(
            (
                i
                for i, token in enumerate(tokens)
                if token.depth == 0 and token.lower == "on"
            ),
            None,
        )
        if trigger_on is not None:
            relation = _relation_after(tokens, trigger_on + 1, "trigger_on", ctes)
            if relation is not None:
                relation_indexes.add(trigger_on)
                relation_indexes.update(relation.indexes)
                relations.append(relation)
    for i, token in enumerate(tokens):
        if token.kind != "ident":
            continue
        context = token.lower
        if context == "into" and (i == 0 or tokens[i - 1].lower != "insert"):
            continue
        if context == "update" and i and tokens[i - 1].lower in {"for", "do"}:
            continue
        # In CREATE TRIGGER grammar, UPDATE OF introduces the list of watched
        # columns; ``OF`` is not the target relation.  Keep this contextual so
        # a genuine table/column named ``of`` is not globally suppressed.
        if (
            context == "update"
            and i + 1 < len(tokens)
            and tokens[i + 1].lower == "of"
            and create_trigger
        ):
            continue
        if context == "from":
            if i and tokens[i - 1].lower == "distinct":
                continue
            if _enclosing_function(tokens, i) in {"extract", "substring", "trim", "overlay"}:
                continue
        if context not in {"from", "join", "update", "into", "using"}:
            continue
        if context == "using" and not any(t.lower == "delete" for t in tokens[:i]):
            continue
        relation = _relation_after(tokens, i + 1, context, ctes)
        if relation is None:
            continue
        relation_indexes.add(i)
        relation_indexes.update(relation.indexes)
        relations.append(relation)
        # Old lorsource queries still use comma-separated FROM lists.  Resolve
        # every relation in that list, while respecting the nesting depth.
        if context == "from":
            j = relation.end
            base_depth = token.depth
            while j < len(tokens) and tokens[j].value == "," and tokens[j].depth == base_depth:
                comma_relation = _relation_after(tokens, j + 1, "from", ctes)
                if comma_relation is None:
                    break
                relation_indexes.add(j)
                relation_indexes.update(comma_relation.indexes)
                relations.append(comma_relation)
                j = comma_relation.end
    return relations, relation_indexes


def _criticality(source: str, symbol: str | None) -> tuple[str, str]:
    name = (symbol or "").lower()
    if source == "src/infra/postgres/topic_repository.rs":
        return "P0", "topic persistence/list/detail"
    if source in {"src/auth.rs", "src/routes/auth.rs"}:
        return "P0", "authentication/session"
    if source == "src/routes/comments.rs":
        return "P0", "comment create/render/moderation"
    if source == "src/routes/topics.rs":
        if any(word in name for word in ("add", "create", "edit", "topic", "section", "index")):
            return "P0", "topic create/list/detail"
        return "P1", "topic browser flow"
    if source == "src/infra/postgres/database.rs":
        return "P0", "startup/schema compatibility"
    if source == "src/search_index.rs":
        return "P1", "search indexing"
    if source.startswith("src/routes/"):
        part = Path(source).stem
        if part in {"groups", "users", "tags", "api", "legacy"}:
            return "P1", f"{part} routes"
        if part in {"admin", "mod"}:
            return "P2", "moderation/admin"
        return "P2", f"{part} routes"
    if source.startswith("src/infra/postgres/"):
        return "P1", "PostgreSQL repository"
    return "P2", "supporting runtime"


def _preview(sql: str, limit: int = 220) -> str:
    compact = re.sub(r"\s+", " ", sql).strip()
    return compact if len(compact) <= limit else compact[: limit - 1] + "…"


def _finding(
    kind: str,
    identifier: str,
    message: str,
    *,
    severity: str,
    table: str | None = None,
    column: str | None = None,
    alias: str | None = None,
    sql_offset: int | None = None,
) -> dict[str, object]:
    return {
        "severity": severity,
        "kind": kind,
        "identifier": identifier,
        "table": table,
        "column": column,
        "alias": alias,
        "message": message,
        "_sql_offset": sql_offset,
    }


def _top_level_segments(tokens: Sequence[Token], start: int, end: int, separator: str = ",") -> list[list[tuple[int, Token]]]:
    if start >= end:
        return []
    base = tokens[start].depth
    segments: list[list[tuple[int, Token]]] = [[]]
    for index in range(start, end):
        token = tokens[index]
        if token.value == separator and token.depth == base:
            segments.append([])
        else:
            segments[-1].append((index, token))
    return [segment for segment in segments if segment]


def _created_table_columns(tokens: Sequence[Token]) -> tuple[str, set[str]] | None:
    """Return columns declared by a simple CREATE TABLE fixture literal.

    This deliberately recognizes only explicit parenthesized definitions.  It
    does not guess columns for CREATE TABLE AS, LIKE, dynamic identifiers or
    table constraints.
    """
    if not tokens or tokens[0].lower != "create":
        return None
    i = 1
    if i < len(tokens) and tokens[i].lower in {"global", "local"}:
        i += 1
    if i < len(tokens) and tokens[i].lower in {"temp", "temporary", "unlogged"}:
        i += 1
    if i >= len(tokens) or tokens[i].lower != "table":
        return None
    i += 1
    if i + 2 < len(tokens) and [token.lower for token in tokens[i:i + 3]] == ["if", "not", "exists"]:
        i += 3
    if i >= len(tokens) or tokens[i].kind != "ident":
        return None
    table = tokens[i].lower
    i += 1
    while i + 1 < len(tokens) and tokens[i].value == "." and tokens[i + 1].kind == "ident":
        table = tokens[i + 1].lower
        i += 2
    if i >= len(tokens) or tokens[i].value != "(":
        return None
    end = _matching_paren(tokens, i)
    constraint_starters = {
        "check", "constraint", "exclude", "foreign", "like", "primary", "unique",
    }
    columns: set[str] = set()
    for segment in _top_level_segments(tokens, i + 1, end):
        first = next((token.lower for _, token in segment if token.kind == "ident"), None)
        if first is not None and first not in constraint_starters:
            columns.add(first)
    return (table, columns) if columns else None


def _postgres_syntax_indexes(tokens: Sequence[Token]) -> set[int]:
    """Token positions which are grammar, not unqualified column names."""
    indexes: set[int] = set()
    for i in range(len(tokens) - 2):
        if [token.lower for token in tokens[i:i + 3]] == ["at", "time", "zone"]:
            indexes.update((i, i + 1, i + 2))
    return indexes


def _insert_columns(tokens: Sequence[Token], relation: Relation) -> list[tuple[int, str]]:
    if relation.context != "into" or relation.table is None:
        return []
    # INSERT target aliases are rare; the column list is the first opening
    # parenthesis after the relation name and before VALUES/SELECT.
    i = relation.end
    if i >= len(tokens) or tokens[i].value != "(":
        return []
    end = _matching_paren(tokens, i)
    columns: list[tuple[int, str]] = []
    for segment in _top_level_segments(tokens, i + 1, end):
        idents = [(index, token.lower) for index, token in segment if token.kind == "ident"]
        if len(idents) == 1:
            columns.append(idents[0])
    return columns


def _update_columns(tokens: Sequence[Token], relation: Relation) -> list[tuple[int, str]]:
    if relation.context != "update" or relation.table is None:
        return []
    set_index = next((i for i in range(relation.end, len(tokens)) if tokens[i].lower == "set"), None)
    if set_index is None:
        return []
    depth = tokens[set_index].depth
    end = len(tokens)
    for i in range(set_index + 1, len(tokens)):
        if tokens[i].depth == depth and tokens[i].lower in {"where", "returning", "from"}:
            end = i
            break
    columns: list[tuple[int, str]] = []
    for segment in _top_level_segments(tokens, set_index + 1, end):
        equals = next((pos for pos, (_, token) in enumerate(segment) if token.value == "="), None)
        if equals is None:
            continue
        lhs = segment[:equals]
        idents = [(index, token.lower) for index, token in lhs if token.kind == "ident"]
        if idents:
            columns.append(idents[-1])
    return columns


def _trigger_update_columns(tokens: Sequence[Token], relation: Relation) -> list[tuple[int, str]]:
    if relation.context != "trigger_on" or relation.table is None:
        return []
    update = next(
        (
            i
            for i, token in enumerate(tokens[:-1])
            if token.depth == 0
            and token.lower == "update"
            and tokens[i + 1].lower == "of"
        ),
        None,
    )
    if update is None:
        return []
    columns: list[tuple[int, str]] = []
    for i in range(update + 2, len(tokens)):
        token = tokens[i]
        if token.depth == 0 and token.lower in {"on", "or"}:
            break
        if token.kind == "ident":
            columns.append((i, token.lower))
    return columns


def _explicit_enum_checks(
    tokens: Sequence[Token],
    relations: Sequence[Relation],
    column_types: dict[tuple[str, str], str],
    enums: dict[str, set[str]],
    severity: str,
) -> list[dict[str, object]]:
    findings: list[dict[str, object]] = []
    seen: set[tuple[str, str]] = set()

    def validate(
        enum_type: str,
        value: str,
        table: str | None = None,
        column: str | None = None,
        sql_offset: int | None = None,
    ) -> None:
        key = (enum_type, value)
        if key in seen:
            return
        seen.add(key)
        if enum_type not in enums:
            findings.append(_finding(
                "missing_enum_type", enum_type,
                f"enum type {enum_type!r} is not present in the current Java schema",
                severity=severity, table=table, column=column, sql_offset=sql_offset,
            ))
        elif value not in enums[enum_type]:
            findings.append(_finding(
                "missing_enum_label", f"{enum_type}.{value}",
                f"enum label {value!r} is not present in Java enum {enum_type}",
                severity=severity, table=table, column=column, sql_offset=sql_offset,
            ))

    # Explicit PostgreSQL casts are the most reliable enum literal signal.
    for i in range(len(tokens) - 2):
        if tokens[i].kind == "string" and tokens[i + 1].value == "::" and tokens[i + 2].kind == "ident":
            enum_type = tokens[i + 2].lower
            if enum_type in enums or enum_type not in BUILTIN_TYPES:
                validate(enum_type, tokens[i].value, sql_offset=tokens[i].offset)

    # Literal assignments to known enum-typed UPDATE columns.
    for relation in relations:
        if relation.context != "update" or relation.table is None:
            continue
        set_index = next((i for i in range(relation.end, len(tokens)) if tokens[i].lower == "set"), None)
        if set_index is None:
            continue
        depth = tokens[set_index].depth
        end = next((i for i in range(set_index + 1, len(tokens)) if tokens[i].depth == depth and tokens[i].lower in {"where", "returning", "from"}), len(tokens))
        for segment in _top_level_segments(tokens, set_index + 1, end):
            eq = next((pos for pos, (_, token) in enumerate(segment) if token.value == "="), None)
            if eq is None:
                continue
            lhs = [token.lower for _, token in segment[:eq] if token.kind == "ident"]
            rhs = [token for _, token in segment[eq + 1:] if token.kind == "string"]
            if lhs and rhs:
                column = lhs[-1]
                enum_type = column_types.get((relation.table, column))
                if enum_type in enums:
                    literal_token = next(token for _, token in segment[eq + 1:] if token.kind == "string")
                    validate(enum_type, rhs[0].value, relation.table, column, literal_token.offset)

    # Literal VALUES aligned with an explicit INSERT column list.
    for relation in relations:
        if relation.context != "into" or relation.table is None:
            continue
        columns = _insert_columns(tokens, relation)
        values_index = next((i for i in range(relation.end, len(tokens)) if tokens[i].lower == "values"), None)
        if values_index is None or values_index + 1 >= len(tokens) or tokens[values_index + 1].value != "(":
            continue
        end = _matching_paren(tokens, values_index + 1)
        values = _top_level_segments(tokens, values_index + 2, end)
        if len(columns) != len(values):
            continue
        for (_, column), expression in zip(columns, values):
            literals = [token.value for _, token in expression if token.kind == "string"]
            enum_type = column_types.get((relation.table, column))
            if enum_type in enums and literals:
                literal_token = next(token for _, token in expression if token.kind == "string")
                validate(enum_type, literals[0], relation.table, column, literal_token.offset)

    # Comparisons between resolved enum columns and SQL string literals.
    aliases: dict[str, set[str]] = defaultdict(set)
    for relation in relations:
        if relation.table and not relation.virtual and not relation.system:
            aliases[relation.table].add(relation.table)
            if relation.alias:
                aliases[relation.alias].add(relation.table)
    comparison_ops = {"=", "!=", "<>", "is"}
    for i in range(len(tokens) - 4):
        if tokens[i].kind == "ident" and tokens[i + 1].value == "." and tokens[i + 2].kind == "ident" and tokens[i + 3].lower in comparison_ops and tokens[i + 4].kind == "string":
            owners = aliases.get(tokens[i].lower, set())
            if len(owners) == 1:
                table = next(iter(owners))
                column = tokens[i + 2].lower
                enum_type = column_types.get((table, column))
                if enum_type in enums:
                    validate(enum_type, tokens[i + 4].value, table, column, tokens[i + 4].offset)

    # An unqualified enum column is safe to resolve when exactly one concrete
    # relation in the statement owns that column.
    concrete_tables = {
        relation.table
        for relation in relations
        if relation.table is not None and not relation.virtual and not relation.system
    }
    if not any(relation.virtual or relation.system for relation in relations):
        for i in range(len(tokens) - 2):
            if tokens[i].kind != "ident" or tokens[i + 1].lower not in comparison_ops or tokens[i + 2].kind != "string":
                continue
            column = tokens[i].lower
            owners = [table for table in concrete_tables if (table, column) in column_types]
            if len(owners) == 1:
                table = owners[0]
                enum_type = column_types[(table, column)]
                if enum_type in enums:
                    validate(enum_type, tokens[i + 2].value, table, column, tokens[i + 2].offset)
    return findings


def audit_sql(
    item: RustString,
    tables: dict[str, set[str]],
    column_types: dict[tuple[str, str], str],
    enums: dict[str, set[str]],
    alias_hints: dict[str, set[str]] | None = None,
    test_fixture_tables: dict[str, set[str]] | None = None,
) -> dict[str, object]:
    canonical_tables = tables
    if item.test_scope and test_fixture_tables:
        # Test-only queries can run against a private schema whose local table
        # intentionally extends a canonical relation.  Merge only columns
        # explicitly declared by CREATE TABLE literals in the same Rust file;
        # production queries continue to use the canonical Java contract.
        tables = {table: set(columns) for table, columns in tables.items()}
        for table, columns in test_fixture_tables.items():
            tables.setdefault(table, set()).update(columns)
    tokens = tokenize_sql(item.value)
    ctes = find_ctes(tokens)
    relations, relation_indexes = find_relations(tokens, ctes)
    syntax_indexes = _postgres_syntax_indexes(tokens)
    criticality, surface = _criticality(item.source, item.symbol)
    findings: list[dict[str, object]] = []
    seen_findings: set[tuple[str, str]] = set()

    def add(finding: dict[str, object]) -> None:
        key = (str(finding["kind"]), str(finding["identifier"]))
        if key not in seen_findings:
            seen_findings.add(key)
            sql_offset = finding.pop("_sql_offset", None)
            if isinstance(sql_offset, int):
                finding["line"] = item.line + item.value.count("\n", 0, sql_offset)
            findings.append(finding)

    aliases: dict[str, set[str]] = defaultdict(set)
    opaque_aliases: set[str] = set()
    unresolved_aliases: set[str] = set()
    for relation in relations:
        if relation.virtual or relation.system:
            if relation.alias:
                opaque_aliases.add(relation.alias)
            if relation.table:
                opaque_aliases.add(relation.table)
            continue
        if relation.table is None:
            continue
        table = relation.table
        aliases[table].add(table)
        if relation.alias:
            aliases[relation.alias].add(table)
        if table not in tables:
            add(_finding(
                "missing_table", table,
                f"relation {table!r} is not present in compat/java-db/schema-contract.tsv",
                severity=criticality, table=table, alias=relation.alias,
                sql_offset=tokens[min(relation.indexes)].offset if relation.indexes else None,
            ))
    insert_target = next((r.table for r in relations if r.context == "into" and r.table in tables), None)
    if insert_target:
        aliases["excluded"].add(insert_target)

    # Qualified columns: alias.column, table.column and EXCLUDED.column.
    qualified_indexes: set[int] = set()
    for i in range(len(tokens) - 2):
        left, dot, right = tokens[i:i + 3]
        if left.kind != "ident" or dot.value != "." or right.kind != "ident":
            continue
        qualified_indexes.update((i, i + 1, i + 2))
        owners = aliases.get(left.lower, set())
        if not owners and left.lower not in opaque_aliases and alias_hints:
            owners = alias_hints.get(left.lower, set())
        if len(owners) == 1:
            table = next(iter(owners))
            if table in tables and right.lower not in tables[table]:
                add(_finding(
                    "missing_column", f"{table}.{right.lower}",
                    f"{left.value}.{right.value} resolves through alias {left.value!r} to missing {table}.{right.lower}",
                    severity=criticality, table=table, column=right.lower, alias=left.lower,
                    sql_offset=left.offset,
                ))
        elif (
            not owners
            and left.lower not in SYSTEM_SCHEMAS
            and left.lower not in opaque_aliases
            and left.lower not in {"new", "old"}
        ):
            unresolved_aliases.add(left.lower)

    # INSERT and UPDATE column positions are unambiguous even without aliases.
    explicit_column_indexes: set[int] = set()
    for relation in relations:
        if relation.table not in tables:
            continue
        explicit = (
            _insert_columns(tokens, relation)
            + _update_columns(tokens, relation)
            + _trigger_update_columns(tokens, relation)
        )
        for index, column in explicit:
            explicit_column_indexes.add(index)
            if column not in tables[relation.table]:
                add(_finding(
                    "missing_column", f"{relation.table}.{column}",
                    f"{column!r} is not a column of canonical table {relation.table}",
                    severity=criticality, table=relation.table, column=column,
                    sql_offset=tokens[index].offset,
                ))

    # Conservative unqualified pass.  It is disabled when a CTE, subquery,
    # table-valued function or system catalog could legally supply columns.
    canonical_relations = {r.table for r in relations if r.table in tables and not r.virtual}
    opaque_sources = any(r.virtual or r.system for r in relations)
    output_aliases: set[str] = set()
    for i, token in enumerate(tokens[:-1]):
        if token.lower == "as" and tokens[i + 1].kind == "ident":
            output_aliases.add(tokens[i + 1].lower)
    if canonical_relations and not opaque_sources and not ctes and not _is_create_trigger(tokens):
        available = set().union(*(tables[table] for table in canonical_relations if table is not None))
        relation_names = set(aliases) | canonical_relations
        skip_after = {"collate", "as"}
        for i, token in enumerate(tokens):
            if (
                token.kind != "ident"
                or i in relation_indexes
                or i in qualified_indexes
                or i in explicit_column_indexes
                or i in syntax_indexes
            ):
                continue
            word = token.lower
            previous = tokens[i - 1] if i else None
            following = tokens[i + 1] if i + 1 < len(tokens) else None
            if (
                word in NON_COLUMN_WORDS
                or word in relation_names
                or word in output_aliases
                or word in ctes
                or (following is not None and following.value == "(")
                or (previous is not None and (previous.value == "::" or previous.lower in skip_after))
                or (following is not None and following.value == "::")
            ):
                continue
            # Bare aliases after a closing expression are not columns.
            if previous is not None and previous.value == ")" and following is not None and following.value in {",", ";"}:
                continue
            if word not in available:
                add(_finding(
                    "missing_unqualified_column", word,
                    f"unqualified identifier {word!r} is absent from all resolved canonical relations: {', '.join(sorted(canonical_relations))}",
                    severity=criticality, column=word, sql_offset=token.offset,
                ))

    for finding in _explicit_enum_checks(tokens, relations, column_types, enums, criticality):
        add(finding)

    # The startup fingerprint deliberately asks PostgreSQL whether removed or
    # Rust-only columns exist.  Inventory those probes separately; their string
    # literals are not runtime column dereferences.
    intentional_probes: list[dict[str, object]] = []
    sql_lower = item.value.lower()
    if "pg_attribute" in sql_lower:
        probe_block = re.search(
            r"\(\s*c\.relname\s*,\s*a\.attname\s*\)\s+in\s*\((.*?)\)\s*\)\s+as\s+has_legacy",
            sql_lower,
            re.DOTALL,
        )
        probe_sql = probe_block.group(1) if probe_block else ""
        for pair in re.finditer(r"\(\s*'([a-z_][\w$]*)'\s*,\s*'([a-z_][\w$]*)'\s*\)", probe_sql):
            table, column = pair.groups()
            if column not in canonical_tables.get(table, set()):
                sql_offset = (probe_block.start(1) if probe_block else 0) + pair.start()
                intentional_probes.append({
                    "table": table,
                    "column": column,
                    "identifier": f"{table}.{column}",
                    "purpose": "negative schema-fingerprint probe (not a column dereference)",
                    "line": item.line + item.value.count("\n", 0, sql_offset),
                })

    query_id = hashlib.sha256(f"{item.source}:{item.line}:{item.value}".encode()).hexdigest()[:12]
    dynamic = bool(re.search(r"\{[A-Za-z_][^}]*\}", item.value))
    fragment = not bool(SQL_START_RE.search(item.value))
    status = "invalid" if findings else ("review" if dynamic or fragment or unresolved_aliases else "clean")
    return {
        "id": query_id,
        "source": item.source,
        "line": item.line,
        "symbol": item.symbol,
        "test_scope": item.test_scope,
        "fixture_schema_relations": sorted({
            relation.table
            for relation in relations
            if item.test_scope
            and test_fixture_tables
            and relation.table in test_fixture_tables
        }),
        "criticality": criticality,
        "runtime_surface": surface,
        "dynamic": dynamic,
        "fragment": fragment,
        "status": status,
        "sql_preview": _preview(item.value),
        "relations": [
            {
                "table": relation.table,
                "alias": relation.alias,
                "schema": relation.schema,
                "context": relation.context,
                "virtual": relation.virtual,
                "system": relation.system,
            }
            for relation in relations
        ],
        "ctes": sorted(ctes),
        "unresolved_qualifiers": sorted(unresolved_aliases - SYSTEM_SCHEMAS),
        "findings": findings,
        "intentional_absence_probes": intentional_probes,
    }


def run_audit(root: Path, schema_contract: Path, java_sql_root: Path) -> dict[str, object]:
    tables, column_types = read_schema_contract(schema_contract)
    enums = read_java_enums(java_sql_root)
    strings = extract_sql_strings(root)
    test_fixture_tables_by_source: dict[str, dict[str, set[str]]] = defaultdict(
        lambda: defaultdict(set)
    )
    for item in strings:
        definition = _created_table_columns(tokenize_sql(item.value))
        if definition is not None:
            table, columns = definition
            test_fixture_tables_by_source[item.source][table].update(columns)
    preliminary = [
        audit_sql(
            item,
            tables,
            column_types,
            enums,
            test_fixture_tables=dict(test_fixture_tables_by_source.get(item.source, {})),
        )
        for item in strings
    ]
    aliases_by_file: dict[tuple[str, str], set[str]] = defaultdict(set)
    aliases_by_symbol: dict[tuple[str, str | None, str], set[str]] = defaultdict(set)
    for query in preliminary:
        if query["fragment"]:
            continue
        for relation in query["relations"]:
            table = relation["table"]
            alias = relation["alias"]
            if table in tables and alias and not relation["virtual"] and not relation["system"]:
                aliases_by_file[(query["source"], alias)].add(table)
                aliases_by_symbol[(query["source"], query["symbol"], alias)].add(table)
    queries = []
    for item in strings:
        hints: dict[str, set[str]] = defaultdict(set)
        for (source, alias), owners in aliases_by_file.items():
            if source == item.source:
                hints[alias].update(owners)
        for (source, symbol, alias), owners in aliases_by_symbol.items():
            if source == item.source and symbol == item.symbol:
                # Same-function evidence is stronger; replace a potentially
                # ambiguous file-wide mapping when it resolves uniquely.
                if len(owners) == 1:
                    hints[alias] = set(owners)
        queries.append(
            audit_sql(
                item,
                tables,
                column_types,
                enums,
                dict(hints),
                dict(test_fixture_tables_by_source.get(item.source, {})),
            )
        )
    findings: list[dict[str, object]] = []
    probes: list[dict[str, object]] = []
    for query in queries:
        for finding in query["findings"]:
            findings.append({
                **finding,
                "query_id": query["id"],
                "source": query["source"],
                "line": finding.get("line", query["line"]),
                "symbol": query["symbol"],
                "criticality": query["criticality"],
                "runtime_surface": query["runtime_surface"],
                "sql_preview": query["sql_preview"],
            })
        for probe in query["intentional_absence_probes"]:
            probes.append({
                **probe,
                "query_id": query["id"],
                "source": query["source"],
                "line": probe.get("line", query["line"]),
                "symbol": query["symbol"],
            })
    rank = {"P0": 0, "P1": 1, "P2": 2, "P3": 3}
    findings.sort(key=lambda row: (rank.get(str(row["criticality"]), 9), str(row["source"]), int(row["line"]), str(row["identifier"])))
    probes.sort(key=lambda row: str(row["identifier"]))
    severity_counts = Counter(str(row["criticality"]) for row in findings)
    kind_counts = Counter(str(row["kind"]) for row in findings)
    status_counts = Counter(str(query["status"]) for query in queries)
    runtime_paths: dict[tuple[str, str], dict[str, object]] = {}
    for query in queries:
        key = (str(query["criticality"]), str(query["runtime_surface"]))
        row = runtime_paths.setdefault(key, {
            "criticality": key[0],
            "runtime_surface": key[1],
            "sql_literals": 0,
            "invalid_queries": 0,
            "findings": 0,
            "review_queries": 0,
        })
        row["sql_literals"] += 1
        row["invalid_queries"] += query["status"] == "invalid"
        row["findings"] += len(query["findings"])
        row["review_queries"] += query["status"] == "review"
    ranked_runtime_paths = sorted(
        runtime_paths.values(),
        key=lambda row: (
            rank.get(str(row["criticality"]), 9),
            -int(row["findings"]),
            str(row["runtime_surface"]),
        ),
    )
    return {
        "generated_from": {
            "rust_root": ".",
            "schema_contract": str(schema_contract.relative_to(root)),
            "java_sql_root": str(java_sql_root.relative_to(root)),
        },
        "schema": {
            "tables": len(tables),
            "columns": sum(len(columns) for columns in tables.values()),
            "enum_types": {name: sorted(values) for name, values in enums.items()},
        },
        "summary": {
            "sql_literals": len(queries),
            "clean_queries": status_counts["clean"],
            "review_queries": status_counts["review"],
            "invalid_queries": status_counts["invalid"],
            "findings": len(findings),
            "findings_by_criticality": dict(sorted(severity_counts.items())),
            "findings_by_kind": dict(sorted(kind_counts.items())),
            "intentional_absence_probes": len(probes),
            "dynamic_queries": sum(bool(query["dynamic"]) for query in queries),
            "sql_fragments": sum(bool(query["fragment"]) for query in queries),
            "test_scope_queries": sum(bool(query["test_scope"]) for query in queries),
            "queries_with_unresolved_qualifiers": sum(bool(query["unresolved_qualifiers"]) for query in queries),
        },
        "findings": findings,
        "intentional_absence_probes": probes,
        "runtime_critical_paths": ranked_runtime_paths,
        "queries": queries,
        "limitations": [
            "Bind values and SQL assembled outside Rust string literals cannot be type-checked statically.",
            "Columns produced by CTEs, subqueries, table-valued functions and system catalogs are not guessed.",
            "Dynamic format fragments are inventoried and their statically visible identifiers are checked; runtime branches still require integration tests.",
            "SQL in Rust test functions is inventoried against explicitly declared local CREATE TABLE fixture columns as well as the canonical schema; production literals never receive that fixture overlay.",
            "A clean static result does not establish query behavior, transaction, authorization or migration parity.",
        ],
    }


def write_csv(path: Path, report: dict[str, object]) -> None:
    fields = [
        "record_class", "criticality", "kind", "identifier", "source", "line", "symbol",
        "runtime_surface", "table", "column", "alias", "message", "purpose", "query_id",
        "sql_preview",
    ]
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(
            stream,
            fieldnames=fields,
            extrasaction="ignore",
            lineterminator="\n",
        )
        writer.writeheader()
        writer.writerows({"record_class": "finding", **row} for row in report["findings"])
        writer.writerows(
            {
                "record_class": "intentional_absence_probe",
                "criticality": "INFO",
                "kind": "legacy_absence_probe",
                **row,
            }
            for row in report["intentional_absence_probes"]
        )


def write_markdown(path: Path, report: dict[str, object]) -> None:
    summary = report["summary"]
    lines = [
        "# Rust SQL vs canonical Java schema",
        "",
        "This is a conservative static identifier audit. It is not a semantic-parity claim and does not execute the queries.",
        "",
        "## Summary",
        "",
        f"- Canonical schema: **{report['schema']['tables']} tables / {report['schema']['columns']} columns**.",
        f"- Rust SQL-bearing literals inspected: **{summary['sql_literals']}** ({summary['dynamic_queries']} dynamic templates; {summary['sql_fragments']} continuation fragments).",
        f"- SQL literals in Rust test scope: **{summary['test_scope_queries']}** (local CREATE TABLE fixture columns apply only there).",
        f"- Queries with confirmed identifier/type violations: **{summary['invalid_queries']}**.",
        f"- Confirmed findings: **{summary['findings']}**.",
        f"- Queries requiring static-review caution: **{summary['review_queries']}**.",
        f"- Intentional negative schema probes: **{summary['intentional_absence_probes']}** (reported separately, not failures).",
        "",
        "## Runtime-critical SQL surfaces",
        "",
        "| Rank | Runtime surface | SQL-bearing literals | Invalid queries | Findings | Review |",
        "|---|---|---:|---:|---:|---:|",
    ]
    for row in report["runtime_critical_paths"]:
        lines.append(
            f"| {row['criticality']} | {row['runtime_surface']} | {row['sql_literals']} | "
            f"{row['invalid_queries']} | {row['findings']} | {row['review_queries']} |"
        )
    lines.extend([
        "",
        "## Confirmed findings, runtime-ranked",
        "",
        "| Rank | Kind | Identifier | Runtime surface | Source |",
        "|---|---|---|---|---|",
    ])
    for finding in report["findings"]:
        symbol = f" `{finding['symbol']}`" if finding.get("symbol") else ""
        lines.append(
            f"| {finding['criticality']} | `{finding['kind']}` | `{finding['identifier']}` | "
            f"{finding['runtime_surface']} | `{finding['source']}:{finding['line']}`{symbol} |"
        )
    if not report["findings"]:
        lines.append("| — | — | — | No confirmed static violations | — |")
    lines.extend([
        "",
        "## Intentional legacy-absence probes",
        "",
        "These identifiers occur as data in the startup schema fingerprint. PostgreSQL is asked whether they exist; they are not dereferenced as columns.",
        "",
        "| Identifier | Source | Purpose |",
        "|---|---|---|",
    ])
    for probe in report["intentional_absence_probes"]:
        lines.append(f"| `{probe['identifier']}` | `{probe['source']}:{probe['line']}` | {probe['purpose']} |")
    lines.extend([
        "",
        "## Static-analysis boundary",
        "",
    ])
    lines.extend(f"- {limitation}" for limitation in report["limitations"])
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path, default=Path("."))
    parser.add_argument("--schema-contract", type=Path)
    parser.add_argument("--java-sql-root", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--csv", type=Path)
    parser.add_argument("--md", type=Path)
    parser.add_argument("--fail-on-findings", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    schema_contract = (args.schema_contract or root / "compat/java-db/schema-contract.tsv").resolve()
    java_sql_root = (args.java_sql_root or root / "compat/java-db/sql").resolve()
    report = run_audit(root, schema_contract, java_sql_root)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.csv:
        write_csv(args.csv, report)
    if args.md:
        write_markdown(args.md, report)
    if not any((args.json, args.csv, args.md)):
        print(json.dumps(report, ensure_ascii=False, indent=2))
    return 1 if args.fail_on_findings and report["summary"]["findings"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
