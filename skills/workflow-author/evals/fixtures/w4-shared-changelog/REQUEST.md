# Request

The request approves the workflow type. Author `changelog.workflow.md`.

Four package workers inspect separate package histories. All workers would otherwise write `drafts/changelog.md`. Remove that shared write from the parallel jobs.

Make each worker return a fixed record. One merge job writes the shared file after all returns arrive. Use fresh checkers, cap the first run at four packages, and flag missing workers. The release test result is the anchor.
