import { VERSION, type ExtensionAPI, type Theme } from "@earendil-works/pi-coding-agent";

function buildHeader(theme: Theme): string[] {
	const logo = [
		"██████   ██",
		"██   ██  ██",
		"██████   ██",
		"██       ██",
		"██       ██",
	].map((line) => theme.bold(theme.fg("accent", line)));
	return ["", ...logo, `${theme.bold("Pi")} ${theme.fg("dim", `v${VERSION}`)}`, theme.fg("muted", "Global tools and personal settings"), ""];
}

export default function customHeader(pi: ExtensionAPI) {
	pi.on("session_start", (_event, ctx) => {
		if (ctx.mode !== "tui") return;
		ctx.ui.setHeader((_tui, theme) => ({
			render: () => buildHeader(theme),
			invalidate() {},
		}));
	});

	pi.registerCommand("builtin-header", {
		description: "Restore the built-in Pi header.",
		handler: (_args, ctx) => ctx.ui.setHeader(undefined),
	});
}
