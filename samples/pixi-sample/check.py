import os
import sys

import numpy

print("python", sys.version.split()[0], "from", sys.prefix)
print("numpy", numpy.__version__, "sum", int(numpy.arange(10).sum()))
print("activation:", os.environ.get("SAMPLE_ACTIVATED_IN"), "|", os.environ.get("SAMPLE_GREETING"))
