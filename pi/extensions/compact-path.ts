import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { homedir } from "node:os";
import { relative } from "node:path";

export function compactPath(path: string, home = homedir()): string {
	const relativePath = relative(home, path);
	const displayPath = relativePath === "" ? "~" : relativePath.startsWith("..") ? path : `~/${relativePath}`;
	const segments = displayPath.split("/");
	return segments.length > 5 ? `…/${segments.slice(-4).join("/")}` : displayPath;
}

export default function compactPathExtension(pi: ExtensionAPI): void {
	pi.on("session_start", (_event, ctx) => {
		if (ctx.mode !== "tui") return;
		ctx.ui.setWidget("compact-path", (_tui, theme) => ({
			render: (width) => [theme.fg("muted", compactPath(ctx.cwd).slice(0, width))],
			invalidate() {},
		}));
	});
}
