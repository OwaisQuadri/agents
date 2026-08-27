import { isToolCallEventType, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { realpathSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { blockedConfigToolCall } from "./config-write-guard/policy.ts";

export default function configWriteGuard(pi: ExtensionAPI): void {
	const extensionPath = realpathSync(fileURLToPath(import.meta.url));
	const repositoryRoot = resolve(dirname(extensionPath), "../..");

	pi.on("resources_discover", () => ({
		skillPaths: [resolve(repositoryRoot, "skills")],
		themePaths: [resolve(repositoryRoot, "pi/themes")],
	}));

	pi.on("tool_call", (event) => {
		if (isToolCallEventType("edit", event) || isToolCallEventType("write", event) || isToolCallEventType("bash", event)) {
			const reason = blockedConfigToolCall(event.toolName, event.input);
			if (reason !== undefined) return { block: true, reason };
		}
	});
}
