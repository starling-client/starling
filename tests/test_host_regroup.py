"""Focused regressions for the replayable host-area move."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    'regroup', Path(__file__).resolve().parents[1] / 'tools/refactors/api-pr1b/regroup.py')
regroup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(regroup)


class HostRegroupTests(unittest.TestCase):
    def test_unknown_methods_require_an_assignment_and_keep_their_bytes(self):
        method = '    /// Keep this documentation.\n    fn probe(&self) -> bool {\n        true\n    }\n'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'probe.rs'
            source = 'pub trait ProbeHost {\n' + method + '}\n'
            path.write_text(source)
            with patch.object(regroup, 'API', root):
                with self.assertRaisesRegex(ValueError, 'probe.probe'):
                    regroup.area_sources(root, {}, {})
                self.assertEqual(path.read_text(), source)
                output, _, mapping = regroup.area_sources(root, {}, {'probe': 'entities'})
                self.assertIn(method, output[root / 'entities.rs'])
                self.assertEqual(mapping[('ProbeHost', 'probe')], 'EntitiesHost')

    def test_merging_backend_overrides_keeps_the_other_tier_default(self):
        source = '''impl mod_api::prelude::OldBackend for World {
    fn read(&self) -> bool {
        true
    }
}
impl mod_api::prelude::OtherBackend for World {
    fn another(&self) -> bool {
        false
    }
}
mod_api::impl_backend_areas!(World, except(old, other));
'''
        old = {'OldBackend': ('old', 'Backend'), 'OtherBackend': ('other', 'Backend'),
               'ClientClientBackend': ('client', 'ClientBackend')}
        mapping = {('OldBackend', 'read'): 'PlayersBackend',
                   ('OtherBackend', 'another'): 'PlayersBackend',
                   ('ClientClientBackend', 'client_read'): 'PlayersClientBackend'}
        result = regroup.rewrite_references(source, old, mapping)
        self.assertEqual(result.count('impl mod_api::prelude::PlayersBackend for World'), 1)
        self.assertIn('fn read(&self)', result)
        self.assertIn('fn another(&self)', result)
        self.assertIn('impl mod_api::prelude::PlayersClientBackend for World {}', result)
        self.assertIn('except(players)', result)

    def test_identically_named_local_doubles_do_not_merge_across_scopes(self):
        source = '''fn first() {
    impl mod_api::prelude::OldHost for Double {
        fn read(&self) -> bool {
            true
        }
    }
    mod_api::impl_host_areas!(Double, except(old));
}
fn second() {
    impl mod_api::prelude::OldHost for Double {
        fn read(&self) -> bool {
            false
        }
    }
    mod_api::impl_host_areas!(Double, except(old));
}
'''
        result = regroup.rewrite_references(source, {'OldHost': ('old', 'Host')},
                                             {('OldHost', 'read'): 'EntitiesHost'})
        self.assertEqual(result.count('impl mod_api::prelude::EntitiesHost for Double'), 2)
        self.assertEqual(result.count('except(entities)'), 2)
        self.assertLess(result.index('true'), result.index('fn second'))
        self.assertGreater(result.index('false'), result.index('fn second'))


if __name__ == '__main__':
    unittest.main()
