# Horizon Fork Launcher (arismoko)

This is the arismoko fork, not the official Horizon launcher. It uses separate
settings, game data and account storage and updates only from arismoko/horizon.

A source-built launcher can install fork builds today. Configured Microsoft
sign-in and signed Nightly Play arrive with the fork launcher; no release date is
announced. Upstream authorized the fork to use its Microsoft application ID,
configured through the fork repository variable. Account and token storage remain
separate. A build without the configured ID cannot sign in; explicitly enabled
unsigned local developer single-player Play remains available. Signed builds
still require an account that owns Minecraft.

Windows: this fork is not Authenticode-signed. SmartScreen may show “Windows
protected your PC” or an unknown-publisher warning. The fork's signed checksums do
not remove SmartScreen warnings.

macOS: this fork is ad-hoc signed, has no Apple Developer ID signature and is not
notarized. Gatekeeper may block it or say Apple cannot check it for malicious
software. A checksum signature does not grant OS trust.

Only list platforms whose packaged install, update and coexistence checks actually
ran. Linux x86_64 and Windows x64 are the initial dry-run targets. Windows ARM64 is
not supported by this packaging workflow. macOS is not yet a dry-run target.
