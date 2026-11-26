"""Import tools modules anew inside one test, for modules that read their settings (the datafix root) at import."""
import importlib
import sys
from unittest import mock

import tools


def fresh_tools(test, *names):
    """Import tools.<name> for each name afresh, in order, and return them. The test's patches (datafix_paths.ROOT)
    are in force while they load; the modules already imported return when the test ends."""
    modules = mock.patch.dict(sys.modules)
    modules.start()
    test.addCleanup(modules.stop)
    for name in names:
        attribute = mock.patch.object(tools, name, create=True)
        attribute.start()
        test.addCleanup(attribute.stop)
        if hasattr(tools, name):
            delattr(tools, name)
        sys.modules.pop(f'tools.{name}', None)
    return [importlib.import_module(f'tools.{name}') for name in names]
