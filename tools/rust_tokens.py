"""Tokens from non-test Rust source for lightweight source analysis tools."""
import re

# Strings/comments are tokens so doc examples and quoted test programs never
# become declarations. Raw Rust strings and nested block comments are supported.
TOKEN = re.compile(r'r(?P<hashes>\#*)".*?"(?P=hashes)|"(?:\\.|[^"\\])*"|//[^\n]*|/\*|\*/|\b\w+\b|[^\s]', re.S)


def tokens(source):
    result, depth = [], 0
    for m in TOKEN.finditer(source):
        value = m.group()
        if value == "/*":
            depth += 1
        elif value == "*/" and depth:
            depth -= 1
        elif not depth:
            result.append((value, m.start()))
    return result


def cfg_requires_test(values):
    """True only if the cfg predicate cannot hold with test disabled.

    Other configuration atoms are unknown; any(test, feature) and not(test)
    therefore remain production code. Nested all/any/not are evaluated.
    """
    def possible(parts):
        if parts == ['test']:
            return {False}
        if len(parts) >= 3 and parts[0] in ('all', 'any', 'not') and parts[1] == '(' and parts[-1] == ')':
            args, start, depth = [], 2, 0
            for j in range(2, len(parts) - 1):
                depth += (parts[j] == '(') - (parts[j] == ')')
                if parts[j] == ',' and not depth:
                    if j > start:
                        args.append(possible(parts[start:j]))
                    start = j + 1
            if start < len(parts) - 1:
                args.append(possible(parts[start:-1]))
            if parts[0] == 'not':
                return {not v for v in args[0]} if len(args) == 1 else {True, False}
            outcomes = {parts[0] == 'all'}
            for arg in args:
                outcomes = {(a and b) if parts[0] == 'all' else (a or b)
                            for a in outcomes for b in arg}
            return outcomes
        return {True, False}
    return True not in possible(values)


def code_tokens(source):
    """Remove examples and test-only modules/functions, retaining attributes."""
    ts = [(v, p) for v, p in tokens(source) if not v.startswith(('//', '"', 'r"', 'r#'))]
    out, i = [], 0
    while i < len(ts):
        end = i + 2
        if [v for v, _ in ts[i:i + 4]] == ['#', '[', 'cfg', '(']:
            while end < len(ts) and ts[end][0] != ']':
                end += 1
        else:
            end = i
        if end > i and cfg_requires_test([v for v, _ in ts[i + 4:end - 1]]):
            i = end + 1
            while i < len(ts) and ts[i][0] not in ('{', ';'):
                i += 1
            if i < len(ts) and ts[i][0] == '{':
                depth = 1
                i += 1
                while i < len(ts) and depth:
                    depth += (ts[i][0] == '{') - (ts[i][0] == '}')
                    i += 1
            else:
                i += 1
        else:
            out.append(ts[i])
            i += 1
    return out
