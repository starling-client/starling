#!/usr/bin/env python3
"""Mechanical extraction of launcher identity literals. Run once from repository root."""
from pathlib import Path

REPLACEMENTS = {
    'account.rs': {'"dev.playhorizon.launcher"': 'crate::identity::KEYCHAIN_SERVICE'},
    'data_location.rs': {'"sunrise-launcher-location"': 'crate::identity::LOCATION_FOLDER'},
    'mac_update.rs': {'"sunrise-launcher"': 'crate::identity::EXECUTABLE'},
    'installer.rs': {
        '"Sunrise Launcher"': 'crate::identity::DISPLAY_NAME',
        '"sunrise-launcher.exe"': 'crate::identity::WINDOWS_EXECUTABLE',
        '"Sunrise"': 'crate::identity::PUBLISHER',
        '"https://playsunrise.net"': 'crate::identity::ABOUT_URL',
        'r"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\SunriseLauncher"': 'crate::identity::UNINSTALL_KEY',
        'r"Software\\Sunrise\\Launcher"': 'crate::identity::INSTALL_KEY',
        '"sunrise-launcher-uninstall-"': 'crate::identity::UNINSTALL_PREFIX',
        '"sunrise-launcher-setup"': 'crate::identity::SETUP_PREFIX',
        '"IMAGENAME eq sunrise-launcher.exe"': 'crate::identity::PROCESS_FILTER',
        '"sunrise-launcher.exe."': 'crate::identity::WINDOWS_EXECUTABLE_PREFIX',
        '"sunrise-launcher"': 'crate::identity::EXECUTABLE',
        'r"Software\\Sunrise"': 'crate::identity::REGISTRY_PARENT',
    },
}
for name, replacements in REPLACEMENTS.items():
    path = Path('launcher/src') / name
    text = path.read_text()
    # Test fixtures are independent inputs, not implementation literals.
    boundary = text.find('#[cfg(test)]')
    if boundary < 0:
        boundary = len(text)
    code, tests = text[:boundary], text[boundary:]
    for old, new in replacements.items():
        assert old in code, (name, old)
        code = code.replace(old, new)
    path.write_text(code + tests)
