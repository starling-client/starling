"""The input-only format for saved parity fuzz scenarios and promoted cases."""
import json
import math
import re
from dataclasses import asdict, dataclass
from string import Formatter

CASE_NAME = re.compile(r'[a-z0-9][a-z0-9_-]{0,99}')
BLOCK_ID = re.compile(r'(?:[a-z0-9_.-]+:)?[a-z0-9_./-]+(?:\[[a-z0-9_.,=-]+\])?')
ENTITY_ID = re.compile(r'(?:[a-z0-9_.-]+:)?[a-z0-9_./-]+')


def integer(value):
    return type(value) is int and -(2**63) <= value < 2**63


def position(value):
    return (isinstance(value, list) and len(value) == 3
            and all(integer(n) or (type(n) is float and math.isfinite(n)) for n in value))


def nbt(value):
    return isinstance(value, dict) and all(
        re.fullmatch(r'[a-zA-Z0-9_.+-]+', key) and isinstance(text, str) and text
        and '\n' not in text and '\r' not in text for key, text in value.items())


def entity_id(value):
    return isinstance(value, str) and ENTITY_ID.fullmatch(value)


def nonnegative_i32(value):
    return type(value) is int and 0 <= value < 2**31


# Required and optional input fields by step. Reports and harness captures are not scenario inputs.
STEP_FIELDS = {
    'summon': ({'type': entity_id, 'at': position, 'n': nonnegative_i32, 'nbt': nbt}, {}),
    'block': ({'block': lambda value: isinstance(value, str) and BLOCK_ID.fullmatch(value), 'at': position},
              {'nbt': lambda value: value is None or nbt(value)}),
    'cmd': ({'cmd': lambda value: isinstance(value, str) and value.startswith('/') and '\r' not in value
             and '\n' not in value}, {'at': lambda value: isinstance(value, list) and all(map(position, value))}),
    'tick': ({'n': nonnegative_i32}, {}),
    'dispense': ({'at': position}, {}),
    'reload': ({}, {}),
    'read_entity': ({'n': nonnegative_i32}, {}),
    'read_near': ({'type': entity_id, 'at': position}, {}),
    'read_items': ({}, {}),
    'read_block': ({'at': position}, {}),
}


def validate_scenario(value):
    """Admit only the scenario envelope and fields that render consumes."""
    if not isinstance(value, dict) or value.keys() != {'family', 'seed', 'steps'}:
        raise ValueError('a scenario needs family, seed and steps')
    if value['family'] not in ('persist', 'spawn', 'smoke') or not integer(value['seed']):
        raise ValueError('invalid scenario family or seed')
    steps = value['steps']
    if not isinstance(steps, list) or not steps:
        raise ValueError('scenario steps must be a nonempty list')
    for index, step in enumerate(steps, 1):
        kind = step.get('k') if isinstance(step, dict) else None
        if not isinstance(kind, str) or kind not in STEP_FIELDS:
            raise ValueError(f'step {index}: unknown step kind')
        required, optional = STEP_FIELDS[kind]
        validators = required | optional
        fields = step.keys() - {'k'}
        if not required.keys() <= fields <= validators.keys():
            raise ValueError(f'step {index}: unexpected or missing fields for {kind}')
        for field in fields:
            if not validators[field](step[field]):
                raise ValueError(f'step {index}: invalid {field}')
        if kind == 'cmd':
            indices = {str(i) for i in range(len(step.get('at', [])))}
            for _, field, spec, conversion in Formatter().parse(step['cmd']):
                if field is not None and (field not in indices or spec or conversion):
                    raise ValueError(f'step {index}: command placeholders must be coordinate indices')
    if not any(step['k'].startswith('read_') for step in steps):
        raise ValueError('a scenario must contain a read')
    return value


@dataclass(frozen=True)
class Case:
    name: str
    family: str
    origin_seed: int
    signature: str | None
    scenario: dict
    original: dict

    @classmethod
    def from_input(cls, name, value):
        if not CASE_NAME.fullmatch(name):
            raise ValueError('case name must be a lowercase slug')
        if isinstance(value, list):
            if len(value) != 1:
                raise ValueError('promote one scenario at a time')
            value, = value
        if isinstance(value, dict) and 'scenario' in value:
            if 'original' not in value:
                raise ValueError('a report group must include its original scenario')
            scenario, original = (validate_scenario(value[key]) for key in ('scenario', 'original'))
            signature = value.get('signature')
            if signature is not None and (not isinstance(signature, str) or not signature.strip()
                                          or not signature.isprintable()):
                raise ValueError('invalid group signature')
        else:
            scenario = original = validate_scenario(value)
            signature = None
        if (scenario['family'], scenario['seed']) != (original['family'], original['seed']):
            raise ValueError('shrunk and original scenarios must have the same family and seed')
        if signature is not None and signature != 'crash' and not signature.startswith(f'{original["family"]}|'):
            raise ValueError('group signature does not match the family')
        return cls(name, original['family'], original['seed'], signature, scenario, original)


def load(directory):
    """Read named cases in order, refusing output fields or metadata that doesn't match the steps."""
    cases = []
    for path in sorted(directory.glob('*.json')):
        try:
            raw = json.loads(path.read_text(encoding='utf-8'))
            case = Case.from_input(path.stem, raw)
            if raw != asdict(case):
                raise ValueError('case fields or metadata do not match its scenarios')
        except ValueError as error:
            raise ValueError(f'{path.name}: {error}') from error
        cases.append(case)
    return cases


def write(directory, case):
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / f'{case.name}.json'
    with path.open('x', encoding='utf-8') as output:
        json.dump(asdict(case), output, indent=2, ensure_ascii=False)
        output.write('\n')
    return path
