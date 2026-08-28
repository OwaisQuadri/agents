# Incident and reproduction record

A roadmap injection omitted a dependency from an existing ticket to a newly added ticket. The saved reproduction exited 1 before the candidate change and showed the missing dependency. It exited 0 after the change and showed the dependency in the stored graph.

# Evaluation summaries

The incumbent and candidate runs contain the same case identifiers and the same score for every case. Each reports mean `7.8`. No current case creates a graph with old items and then injects a new prerequisite for one of those old items.

# Pass state

The current worker proposed the source mutation. The evaluation inventory has not changed, and no other worker assignment is recorded yet.
