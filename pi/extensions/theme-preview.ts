import type { ExtensionAPI, ExtensionCommandContext, Theme, ThemeColor } from "@earendil-works/pi-coding-agent";
import { Key, matchesKey, truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";

type BackgroundRole =
	| "selectedBg"
	| "scrollbarThumb"
	| "searchMatchBg"
	| "userMessageBg"
	| "customMessageBg"
	| "toolPendingBg"
	| "toolSuccessBg"
	| "toolErrorBg";

type Sample =
	| { role: ThemeColor; isBackground: false }
	| { role: BackgroundRole; isBackground: true };

type Group = {
	name: string;
	samples: Sample[];
};

type Page = {
	groupName: string;
	samples: Sample[];
};

const SAMPLES_PER_PAGE = 10;

function foreground(...roles: ThemeColor[]): Sample[] {
	return roles.map((role) => ({ role, isBackground: false }));
}

function background(...roles: BackgroundRole[]): Sample[] {
	return roles.map((role) => ({ role, isBackground: true }));
}

const groups: Group[] = [
	{
		name: "Chrome / status",
		samples: foreground(
			"accent",
			"border",
			"borderAccent",
			"borderMuted",
			"success",
			"error",
			"warning",
			"muted",
			"dim",
			"text",
			"thinkingText",
			"searchMatchText",
			"bashMode",
		),
	},
	{
		name: "Chrome / status backgrounds",
		samples: background("selectedBg", "scrollbarThumb", "searchMatchBg"),
	},
	{
		name: "Messages / tools",
		samples: foreground(
			"userMessageText",
			"customMessageText",
			"customMessageLabel",
			"toolTitle",
			"toolOutput",
			"toolDiffAdded",
			"toolDiffRemoved",
			"toolDiffContext",
		),
	},
	{
		name: "Messages / tools backgrounds",
		samples: background("userMessageBg", "customMessageBg", "toolPendingBg", "toolSuccessBg", "toolErrorBg"),
	},
	{
		name: "Markdown",
		samples: foreground(
			"mdHeading",
			"mdLink",
			"mdLinkUrl",
			"mdCode",
			"mdCodeBlock",
			"mdCodeBlockBorder",
			"mdQuote",
			"mdQuoteBorder",
			"mdHr",
			"mdListBullet",
		),
	},
	{
		name: "Syntax",
		samples: foreground(
			"syntaxComment",
			"syntaxKeyword",
			"syntaxFunction",
			"syntaxVariable",
			"syntaxString",
			"syntaxNumber",
			"syntaxType",
			"syntaxOperator",
			"syntaxPunctuation",
		),
	},
	{
		name: "Thinking levels",
		samples: foreground(
			"thinkingOff",
			"thinkingMinimal",
			"thinkingLow",
			"thinkingMedium",
			"thinkingHigh",
			"thinkingXhigh",
			"thinkingMax",
		),
	},
];

function pagesFromGroups(source: readonly Group[]): Page[] {
	return source.flatMap((group) => {
		const pages: Page[] = [];
		for (let index = 0; index < group.samples.length; index += SAMPLES_PER_PAGE) {
			pages.push({ groupName: group.name, samples: group.samples.slice(index, index + SAMPLES_PER_PAGE) });
		}
		return pages;
	});
}

const pages = pagesFromGroups(groups);

class ThemePreview {
	private pageIndex = 0;

	constructor(
		private readonly theme: Theme,
		private readonly requestRender: () => void,
		private readonly close: () => void,
	) {}

	handleInput(data: string): void {
		if (matchesKey(data, Key.escape)) {
			this.close();
			return;
		}
		if (matchesKey(data, Key.left) || matchesKey(data, Key.up) || matchesKey(data, Key.pageUp)) {
			this.pageIndex = Math.max(0, this.pageIndex - 1);
		} else if (matchesKey(data, Key.right) || matchesKey(data, Key.down) || matchesKey(data, Key.pageDown)) {
			this.pageIndex = Math.min(pages.length - 1, this.pageIndex + 1);
		} else if (matchesKey(data, Key.home)) {
			this.pageIndex = 0;
		} else if (matchesKey(data, Key.end)) {
			this.pageIndex = pages.length - 1;
		} else {
			return;
		}
		this.requestRender();
	}

	render(width: number): string[] {
		if (width < 4) return [truncateToWidth("Theme preview", width)];
		const page = pages[this.pageIndex]!;
		const innerWidth = width - 2;
		const pad = (text: string) => text + " ".repeat(Math.max(0, innerWidth - visibleWidth(text)));
		const row = (text: string) => `${this.theme.fg("border", "│")}${pad(truncateToWidth(text, innerWidth, ""))}${this.theme.fg("border", "│")}`;
		const sample = ({ role, isBackground }: Sample) =>
			isBackground
				? this.theme.bg(role, this.theme.fg("text", ` ${role}  background `))
				: this.theme.fg(role, ` ${role}  sample `);

		return [
			this.theme.fg("border", `╭${"─".repeat(innerWidth)}╮`),
			row(` ${this.theme.fg("accent", this.theme.bold(`Theme preview: ${this.theme.name ?? "active"}`))}`),
			row(` ${this.theme.fg("muted", page.groupName)} ${this.theme.fg("dim", `(${this.pageIndex + 1}/${pages.length})`)}`),
			row(""),
			...page.samples.map((entry) => row(` ${sample(entry)}`)),
			row(""),
			row(` ${this.theme.fg("dim", "←/↑ PgUp previous  →/↓ PgDn next  Home/End  Esc close")}`),
			this.theme.fg("border", `╰${"─".repeat(innerWidth)}╯`),
		];
	}

	invalidate(): void {}
}

export default function themePreview(pi: ExtensionAPI): void {
	pi.registerCommand("theme-preview", {
		description: "Preview every role in the active Pi theme.",
		handler: async (_args: string, ctx: ExtensionCommandContext) => {
			if (ctx.mode !== "tui") {
				ctx.ui.notify("Theme preview requires interactive mode.", "error");
				return;
			}
			await ctx.ui.custom<void>(
				(tui, theme, _keybindings, done) => new ThemePreview(theme, () => tui.requestRender(), () => done()),
				{
					overlay: true,
					overlayOptions: { anchor: "center", width: "75%", minWidth: 42, maxHeight: "90%", margin: 1 },
				},
			);
		},
	});
}
