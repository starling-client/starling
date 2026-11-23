#!/usr/bin/env python3
"""Mechanical repository-selection adaptation of the original launcher workflow."""
from pathlib import Path
p = Path('.github/workflows/launcher.yml')
s = p.read_text()
start = s.index('      matrix:\n')
end = s.index('    runs-on:', start)
s = s[:start] + '      matrix: ${{ fromJSON(needs.decide.outputs.matrix) }}\n' + s[end:]
s = s.replace('  SUNRISE_TELEMETRY_URL: "off"', '''  SUNRISE_TELEMETRY_URL: "off"
  SUNRISE_LAUNCHER_REPOSITORY: ${{ github.repository }}
  LAUNCHER_PREFIX: ${{ github.repository == 'arismoko/horizon' && 'sunrise-fork-launcher' || 'sunrise-launcher' }}
  LAUNCHER_INSTALL_FOLDER: ${{ github.repository == 'arismoko/horizon' && 'Sunrise Fork Launcher (arismoko)' || 'Sunrise Launcher' }}
  LAUNCHER_PACKAGING: ${{ github.repository == 'arismoko/horizon' && '../.local/fork-launcher-packaging' || '../packaging' }}''')
s = s.replace('      release: ${{ steps.decide.outputs.release }}', '      matrix: ${{ steps.matrix.outputs.matrix }}\n      release: ${{ steps.decide.outputs.release }}')
s = s.replace('      - id: mode', '      - id: matrix\n        run: echo "matrix=$(python3 tools/launcher_build_matrix.py)" >> "$GITHUB_OUTPUT"\n      - id: mode', 1)
s = s.replace("(vars.MAC_RUNNER || 'ludovics-mac-mini')", "(github.repository == 'arismoko/horizon' && 'macos-latest' || vars.MAC_RUNNER || 'ludovics-mac-mini')")
anchor = '      - name: Only system libraries'
# Rename only the fork artifact; upstream Cargo and packaging names remain identical.
step = '''      - name: Stage repository-selected packaging and executable
        run: |
          if [[ "$SUNRISE_LAUNCHER_REPOSITORY" == arismoko/horizon ]]; then
            python3 ../tools/stage_launcher_packaging.py "$LAUNCHER_PACKAGING" --repository "$SUNRISE_LAUNCHER_REPOSITORY"
          fi
'''
# The DLL and version smoke need the final executable name, so stage before them.
pos = s.index('      - name:', s.index('      - run: cargo build'))
s = s[:pos] + step + '''      - name: Name the fork executable
        if: github.repository == 'arismoko/horizon'
        run: mv "$BIN/sunrise-launcher${{ runner.os == 'Windows' && '.exe' || '' }}" "$BIN/${{ matrix.binary }}"
''' + s[pos:]
s = s.replace('"sunrise-launcher $expected"', '"$LAUNCHER_PREFIX $expected"').replace('::error::expected sunrise-launcher $expected', '::error::expected $LAUNCHER_PREFIX $expected')
s = s.replace('$RUNNER_TEMP/sunrise-launcher-setup.exe', '$RUNNER_TEMP/$LAUNCHER_PREFIX-setup.exe')
s = s.replace('$setup/Sunrise Launcher/sunrise-launcher.exe', '$setup/$LAUNCHER_INSTALL_FOLDER/$LAUNCHER_PREFIX.exe')
s = s.replace('../packaging/macos/package.sh', '"$LAUNCHER_PACKAGING/macos/package.sh"').replace('../packaging/linux/appimage.sh', '"$LAUNCHER_PACKAGING/linux/appimage.sh"')
s = s.replace('pattern: sunrise-launcher-*', 'pattern: "*launcher-*"')
s = s.replace('dist/sunrise-launcher-*', 'dist/"$LAUNCHER_PREFIX"-*')
s = s.replace('for file in sunrise-launcher-windows-arm64.zip sunrise-launcher-setup-arm64.exe;', 'for file in "$LAUNCHER_PREFIX-windows-arm64.zip" "$LAUNCHER_PREFIX-setup-arm64.exe";')
# Every launcher invocation, including tests, gets isolated OS integration locations.
s = s.replace('      # Every test target', '''      - name: Isolate launcher OS integration
        run: |
          for name in SUNRISE_HOME SUNRISE_LAUNCHER_HOME XDG_DATA_HOME XDG_CONFIG_HOME SUNRISE_LAUNCHER_TRASH SUNRISE_LAUNCHER_CLIPBOARD SUNRISE_LAUNCHER_DESKTOP; do
            path="${RUNNER_TEMP//\\\\//}/launcher-isolation/$name"
            mkdir -p "$path"
            echo "$name=$path" >> "$GITHUB_ENV"
          done
          echo "SUNRISE_LAUNCHER_NO_SELF_UPDATE=1" >> "$GITHUB_ENV"
          echo "SUNRISE_NO_AUDIO=1" >> "$GITHUB_ENV"
      # Every test target''')
p.write_text(s)
