Workflow.

PRESERVE-DEPENDENCY-AUDIT-73

GOAL: Audit eight packages for undeclared dependencies.
FAN OUT: One worker inspects each package.
MERGE: Synthesize all returned findings, even when some workers do not return.
CAP: Eight packages on the first run.
SAVE: dependency-audit.md.
