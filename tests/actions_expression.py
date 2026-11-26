"""Syntax and conservative branch analysis for Actions workflow policy tests."""
from dataclasses import dataclass
import json
import math
import re


_STRING = r"'(?:[^']|'')*'"
_TEMPLATE = re.compile(r"\$\{\{((?:" + _STRING + r"|[^'}]|\}(?!\}))*)\}\}")
_TOKEN = re.compile(
    rf"(?P<string>{_STRING})|(?P<number>-?(?:0[xX][0-9a-fA-F]+|(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?))"
    r"|(?P<name>[A-Za-z_][A-Za-z0-9_-]*)|(?P<operator>\|\||&&|==|!=|<=|>=|[!<>.,*()\[\]])|(?P<space>\s+)"
)
_COMPARISONS = {"==", "!=", "<", "<=", ">", ">="}
_PRECEDENCE = {"||": 1, "&&": 2, "==": 3, "!=": 3, "<": 4, "<=": 4, ">": 4, ">=": 4}
_LITERALS = {"true": True, "false": False, "null": None}
_UNKNOWN_TRUTHY = object()
_UNKNOWN_FALSY = object()


@dataclass(frozen=True)
class Expression:
    operator: str
    value: str | int | float | bool | None = None
    operands: tuple["Expression", ...] = ()

    def walk(self):
        yield self
        for operand in self.operands:
            yield from operand.walk()

    def comparisons(self):
        return {node for node in self.walk() if node.operator in _COMPARISONS}


class _Parser:
    def __init__(self, text):
        self.text = text
        self.tokens = []
        offset = 0
        while offset < len(text):
            token = _TOKEN.match(text, offset)
            if token is None:
                raise ValueError(f"invalid Actions expression at {text[offset:]!r} in {text!r}")
            if token.lastgroup != "space":
                self.tokens.append((token.lastgroup, token[0]))
            offset = token.end()
        self.tokens.append(("end", ""))
        self.index = 0

    def peek(self):
        return self.tokens[self.index][1]

    def take(self, expected=None):
        kind, token = self.tokens[self.index]
        if kind == "end" or (expected is not None and token != expected):
            raise ValueError(f"expected {expected or 'an operand'} at {token or 'end of input'!r} in {self.text!r}")
        self.index += 1
        return kind, token

    def expression(self, minimum=1):
        kind, token = self.take()
        if token == "!":
            node = Expression("!", operands=(self.expression(5),))
        elif token == "(":
            node = self.expression()
            self.take(")")
        elif kind == "string":
            node = Expression("literal", token[1:-1].replace("''", "'"))
        elif kind == "number":
            number = int(token, 16) if token.lower().lstrip("-").startswith("0x") else json.loads(token)
            node = Expression("literal", number)
        elif kind == "name":
            name = token.lower()
            if name in _LITERALS:
                node = Expression("literal", _LITERALS[name])
            elif self.peek() == "(":
                self.take("(")
                args = []
                if self.peek() != ")":
                    args.append(self.expression())
                    while self.peek() == ",":
                        self.take(",")
                        args.append(self.expression())
                self.take(")")
                node = Expression("call", name, tuple(args))
            else:
                node = Expression("name", name)
        else:
            raise ValueError(f"expected an operand at {token!r} in {self.text!r}")

        while (operator := self.peek()) in (".", "["):
            self.take()
            if operator == ".":
                kind, property_name = self.take()
                if property_name == "*":
                    node = Expression(".*", operands=(node,))
                    continue
                if kind != "name":
                    raise ValueError(f"expected a property at {property_name!r} in {self.text!r}")
                key = Expression("literal", property_name)
            else:
                key = self.expression()
                self.take("]")
            if key.operator == "literal" and isinstance(key.value, str):
                key = Expression("literal", key.value.lower())
            node = Expression("index", operands=(node, key))

        while (priority := _PRECEDENCE.get(self.peek(), 0)) >= minimum:
            _, operator = self.take()
            node = Expression(operator, operands=(node, self.expression(priority + 1)))
        return node


def parsed_expression(text):
    """Read a complete bare condition or a single ${{ ... }} expression."""
    body = text.strip()
    if body.startswith("${{") and body.endswith("}}"):
        body = body[3:-2]
    parser = _Parser(body)
    result = parser.expression()
    if parser.peek():
        raise ValueError(f"unexpected token {parser.peek()!r} in {text!r}")
    return result


def embedded_expressions(text):
    """Read expressions in template values such as env strings and runner labels."""
    offset = 0
    while (offset := text.find("${{", offset)) != -1:
        match = _TEMPLATE.match(text, offset)
        if match is None:
            raise ValueError(f"unterminated Actions expression at {text[offset:]!r}")
        yield parsed_expression(match[1])
        offset = match.end()


def truthy(value):
    return value is not _UNKNOWN_FALSY and (isinstance(value, (list, dict)) or bool(value))


def branch_values(expression, facts=None):
    """Possible results of short-circuit branches, without guessing dynamic values.

    Comparisons are unknown booleans unless the caller supplies a fact about one;
    of the function calls, only fromJSON on a known string produces a known result,
    and indexing a known value gives the elements its key can pick (null when absent).
    """
    op = expression.operator
    if op == "literal":
        return [expression.value]
    if op in _COMPARISONS:
        return [facts[expression]] if facts and expression in facts else [False, True]
    if op == "!":
        return [not truthy(value) for value in branch_values(expression.operands[0], facts)]
    if op in ("&&", "||"):
        left, right = expression.operands
        values = branch_values(left, facts)
        kept = [value for value in values if truthy(value) == (op == "||")]
        return kept if len(kept) == len(values) else kept + branch_values(right, facts)
    if op == "call" and expression.value == "fromjson" and len(expression.operands) == 1:
        values = branch_values(expression.operands[0], facts)
        decoded = [json.loads(value) for value in values if isinstance(value, str)]
        if len(decoded) == len(values):
            return decoded
        return decoded + [_UNKNOWN_FALSY, _UNKNOWN_TRUTHY]
    if op == "index":
        base, key = expression.operands
        keys = branch_values(key, facts)
        values = [item for value in branch_values(base, facts) for index in keys for item in _elements(value, index)]
        known = [value for value in values if value is not _UNKNOWN]
        return known if len(known) == len(values) else known + [_UNKNOWN_FALSY, _UNKNOWN_TRUTHY]
    return [_UNKNOWN_FALSY, _UNKNOWN_TRUTHY]


_UNKNOWN = object()


def _elements(value, key):
    """What `value[key]` can be in Actions: a list takes a key that is a number, floored; an object a
    string, compared without case. A key the analysis cannot resolve that way (a dynamic value, or a
    string or other value Actions would convert) can give any element; a dynamic value gives _UNKNOWN."""
    if value in (_UNKNOWN_FALSY, _UNKNOWN_TRUTHY):
        return [_UNKNOWN]
    if isinstance(value, list) and isinstance(key, (int, float)) and not isinstance(key, bool):
        index = math.floor(key)
        return [value[index] if 0 <= index < len(value) else None]
    if isinstance(value, dict) and isinstance(key, str):
        return [next((item for name, item in value.items() if name.lower() == key.lower()), None)]
    if isinstance(value, (list, dict)):
        return [*(value if isinstance(value, list) else value.values()), None]
    return [None]
