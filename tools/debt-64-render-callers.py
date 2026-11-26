#!/usr/bin/env python3
"""Route world hooks through the native-replaceable Renderer::render entry point."""
from pathlib import Path
import re
import subprocess

pattern = r'(renderer\.render\(|game\.renderer\.as_mut\(\)\.unwrap\(\)\.render\()(&?frame|&loading_frame\(&(?:hud|batch)\))\)'
for path in subprocess.check_output(
    ['git', 'ls-files', 'crates/sunrise-client', 'crates/sunrise-client-render'], text=True
).splitlines():
    if not path.endswith('.rs'):
        continue
    path = Path(path)
    before = path.read_text()
    after = re.sub(pattern, r'\1\2, None)', before)
    after = after.replace('renderer.render_with_hooks(frame, hooks)', 'renderer.render(frame, hooks)')
    after = after.replace('`Renderer::render_with_hooks`', '`Renderer::render`')
    if before != after:
        path.write_text(after)

path = Path('crates/sunrise-client-render/src/gfx.rs')
before = path.read_text()
wrapper = '''    #[sunrise_mod_api_macros::replaceable]
    pub fn render(&mut self, frame: &Frame) -> Result<bool> {
        self.render_with_hooks(frame, None)
    }
'''
if wrapper in before:
    before = before.replace(wrapper, '')
    before = before.replace('    pub fn render_with_hooks(', '    #[sunrise_mod_api_macros::replaceable]\n    pub fn render(')
    path.write_text(before)
