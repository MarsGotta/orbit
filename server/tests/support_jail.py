"""A process-prefix double; isolation itself is tested only with real bwrap."""

import os
import sys

args = sys.argv[1:]
# The production builder terminates its options with --.
separator = args.index("--")
command = args[separator + 1 :]
if "--chdir" in args[:separator]:
    os.chdir(args[args.index("--chdir") + 1])
os.execv(command[0], command)
