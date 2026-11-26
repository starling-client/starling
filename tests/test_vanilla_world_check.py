import unittest

from tools.vanilla_world_check import log_problems


class VanillaWorldCheckTests(unittest.TestCase):
    def test_ignores_offline_startup_noise_but_not_oracle_failures(self):
        discovery = '[12:00:00] [Minecraft Services Discovery/WARN]: Failed to connect'
        unavailable = 'com.mojang.authlib.exceptions.MinecraftClientException: Services are unavailable'
        startup = [
            discovery,
            'com.mojang.authlib.exceptions.MinecraftClientException: Failed to read from '
            'https://discovery.minecraftservices.com/minecraft/client due to discovery.minecraftservices.com',
            'Caused by: java.net.UnknownHostException: discovery.minecraftservices.com',
            discovery,
            unavailable,
            '[12:00:01] [Server thread/WARN]: Failed to find a usable hardware address from the network interfaces; '
            'using random bytes: 01:23:45:67:89:ab:cd:ef',
        ]
        failures = [
            '[12:00:02] [Server thread/ERROR]: Failed to save chunk',
            '[12:00:02] [Server thread/WARN]: Failed to connect',
            '[12:00:03] [Server thread/WARN]: Failed to find a usable hardware address from the network interfaces; '
            'using random bytes: 01:23:45:67:89:ab:cd:ef (network failure)',
            'login failed: disconnected',
        ]
        self.assertEqual(log_problems(startup + failures), failures)
        other_host = 'Caused by: java.net.UnknownHostException: example.invalid'
        self.assertEqual(log_problems([discovery, other_host]), [other_host])
        continued = [
            discovery,
            unavailable,
            '\tat java.base/java.net.UnknownHostException.example(Source.java:42)',
            '\t... 2 more',
            'background worker stopped',
            unavailable,
        ]
        self.assertEqual(log_problems(continued), [unavailable])

    def test_ignores_windows_system_report_noise_but_not_server_failures(self):
        stack = '\tat net.minecraft.SystemReport.ignoreErrors(SystemReport.java:121)'
        process_null = ('java.lang.NullPointerException: Cannot invoke "oshi.software.os.OSProcess.getUserTime()" '
                        'because "process" is null')
        startup = [
            '[04:37:38] [ServerMain/ERROR]: Unable to locate English counter names in registry Perflib 009. '
            'Counters may need to be rebuilt: ',
            'com.sun.jna.platform.win32.Win32Exception: The parameter is incorrect.',
            '\tat com.sun.jna.platform.win32.Advapi32Util.registryGetStringArray(Advapi32Util.java:921)',
            stack,
            '[04:37:38] [ServerMain/WARN]: Unable to locate English counter names in registry Perflib 009. '
            'Assuming English counters. Error 0x80070057. See https://support.microsoft.com/en-us/help/300956/'
            'how-to-manually-rebuild-performance-counter-library-values',
            '[04:37:38] [ServerMain/WARN]: Failed to get system info for Process Loads',
            process_null,
            stack,
            '[04:37:38] [ServerMain/WARN]: Failed to get system info for Process Virtual Size (MiB)',
            process_null.replace('getUserTime', 'getVirtualSize'),
            stack,
        ]
        failures = [
            '[04:37:41] [Server thread/ERROR]: Failed to save chunk',
            '[04:37:41] [ServerMain/WARN]: Failed to get system info for Process Loads: disk full',
        ]
        self.assertEqual(log_problems(startup + failures), failures)
        # Only the system report's own exceptions belong to its warnings: another one, and its stack, is reported.
        other = 'java.lang.IllegalStateException: Failed to load registry'
        warning = '[04:37:38] [ServerMain/WARN]: Failed to get system info for Process Loads'
        self.assertEqual(log_problems([warning, other, stack]), [other, stack])
