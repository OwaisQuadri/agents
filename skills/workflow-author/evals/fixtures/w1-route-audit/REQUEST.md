# Request

The request approves the workflow type. Author `route-audit.workflow.md` for an audit of 12 route files.

Use one worker for each file. Each worker returns the route path, the authentication result, and test evidence. A fresh checker verifies each finding against the route file and the test result. The workflow reports all missing workers. The first run stops at 12 files.
