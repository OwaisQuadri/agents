import type { ExtensionAPI, ExtensionCommandContext, Theme } from "@earendil-works/pi-coding-agent";
import { Key, matchesKey, truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";

type Spinner = {
	name: string;
	frames: string[];
	intervalMs: number;
};

type PreviewPage =
	| { kind: "spinners"; startIndex: number }
	| { kind: "interface" }
	| { kind: "signal" };

type PreInputPhase = "idle" | "streaming";

type PreInputState = {
	phase: PreInputPhase;
	phaseElapsedMilliseconds: number;
	activeElapsedMilliseconds: number;
};

const SPINNERS: Spinner[] = [
	{
		name: "Braille orbit",
		frames: ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
		intervalMs: 80,
	},
	{
		name: "Dot wave",
		frames: ["●····", "·●···", "··●··", "···●·", "····●", "···●·", "··●··", "·●···"],
		intervalMs: 120,
	},
	{
		name: "Arc orbit",
		frames: ["◜", "◠", "◝", "◡", "◟", "◢", "◞", "◣"],
		intervalMs: 100,
	},
	{
		name: "Pulse / bar wave",
		frames: ["▁▂▄▆█▆▄", "▂▄▆█▆▄▂", "▄▆█▆▄▂▁", "▆█▆▄▂▁▁", "█▆▄▂▁▁▁", "▆▄▂▁▁▁▂", "▄▂▁▁▁▂▄", "▂▁▁▁▂▄▆"],
		intervalMs: 140,
	},
	{
		name: "Arctic Aurora sweep",
		frames: ["·········", "·········", "·········", "·········", "·········", "·········", "·········"],
		intervalMs: 90,
	},
	{
		name: "Pendulum",
		frames: ["●    ", " ●   ", "  ●  ", "   ● ", "    ●", "   ● ", "  ●  ", " ●   "],
		intervalMs: 110,
	},
	{
		name: "Firefly trail",
		frames: ["✦····", "·✦···", "··✦··", "···✦·", "····✦", "···✧·", "··✧··", "·✧···"],
		intervalMs: 120,
	},
	{
		name: "Weaving needle",
		frames: ["╲╱···", "·╲╱··", "··╲╱·", "···╲╱", "···╱╲", "··╱╲·", "·╱╲··", "╱╲···"],
		intervalMs: 105,
	},
	{
		name: "Signal scanner",
		frames: ["┤▏····", "┤·▎···", "┤··▌··", "┤···▊·", "┤····█", "┤···▊·", "┤··▌··", "┤·▎···"],
		intervalMs: 95,
	},
	{
		name: "Constellation",
		frames: ["✦ · ·", "✦ ✧ ·", "✦ ✧ ✦", "✧ ✦ ✧", "· ✦ ✧", "· ✧ ✦"],
		intervalMs: 180,
	},
	{
		name: "Orbiting brackets",
		frames: ["(···)", "[···]", "{···}", "<···>", "{···}", "[···]"],
		intervalMs: 130,
	},
	{
		name: "Tidal ring",
		frames: ["◌", "◔", "◑", "◕", "●", "◕", "◑", "◔"],
		intervalMs: 125,
	},
	{
		name: "Comet",
		frames: ["☄····", "·☄···", "··☄··", "···☄·", "····☄", "···✦·", "··✧··", "·✧···"],
		intervalMs: 100,
	},
	{
		name: "Aurora ribbon",
		frames: ["▁▂▃▄▃▂▁", "▂▃▄▅▄▃▂", "▃▄▅▆▅▄▃", "▄▅▆▇▆▅▄", "▃▄▅▆▅▄▃", "▂▃▄▅▄▃▂"],
		intervalMs: 150,
	},
	{
		name: "Split-flap",
		frames: ["[ A ]", "[ B ]", "[ C ]", "[ D ]", "[ E ]", "[ F ]"],
		intervalMs: 180,
	},
];

const SPINNERS_PER_PAGE = 8;
const PRE_INPUT_IDLE_MILLISECONDS = 900;
const PRE_INPUT_STREAM_MILLISECONDS = 2_400;
const PRE_INPUT_CYCLE_MILLISECONDS = PRE_INPUT_IDLE_MILLISECONDS + PRE_INPUT_STREAM_MILLISECONDS;
const PRE_INPUT_MODEL_LABELS = ["openai-codex/gpt-5.6-terra (high)", "gpt-5.6-terra (high)", "gpt-5.6-terra"];
const ENCRYPTED_CHARACTERS = "0123456789ABCDEF!@#$%^&*+-=<>?/";

function createPages(): PreviewPage[] {
	const pages: PreviewPage[] = [];
	for (let startIndex = 0; startIndex < SPINNERS.length; startIndex += SPINNERS_PER_PAGE) {
		pages.push({ kind: "spinners", startIndex });
	}
	pages.push({ kind: "interface" });
	pages.push({ kind: "signal" });
	return pages;
}

const PAGES = createPages();

class SpinnerPreview {
	private pageIndex = 0;
	private readonly startedAt = Date.now();
	private readonly timer: ReturnType<typeof setInterval>;
	private isDisposed = false;

	constructor(
		private readonly theme: Theme,
		private readonly requestRender: () => void,
		private readonly close: () => void,
	) {
		this.timer = setInterval(() => this.requestRender(), 40);
	}

	handleInput(data: string): void {
		if (matchesKey(data, Key.escape)) {
			this.close();
			return;
		}
		if (matchesKey(data, Key.left) || matchesKey(data, Key.up) || matchesKey(data, Key.pageUp)) {
			this.pageIndex = Math.max(0, this.pageIndex - 1);
		} else if (matchesKey(data, Key.right) || matchesKey(data, Key.down) || matchesKey(data, Key.pageDown)) {
			this.pageIndex = Math.min(PAGES.length - 1, this.pageIndex + 1);
		} else if (matchesKey(data, Key.home)) {
			this.pageIndex = 0;
		} else if (matchesKey(data, Key.end)) {
			this.pageIndex = PAGES.length - 1;
		} else {
			return;
		}
		this.requestRender();
	}

	render(width: number): string[] {
		if (width < 4) return [truncateToWidth("Spinner preview", width)];

		const innerWidth = width - 2;
		const row = (content: string) => {
			const clipped = truncateToWidth(content, innerWidth, "");
			return `${this.theme.fg("border", "│")}${clipped}${" ".repeat(Math.max(0, innerWidth - visibleWidth(clipped)))}${this.theme.fg("border", "│")}`;
		};
		const page = PAGES[this.pageIndex]!;
		const content = page.kind === "spinners"
			? this.renderSpinnerPage(page, row)
			: page.kind === "interface"
				? this.renderInterfacePage(row)
				: this.renderPreInputSignalPage(row, innerWidth);

		return [
			this.theme.fg("border", `╭${"─".repeat(innerWidth)}╮`),
			...content,
			row(""),
			row(` ${this.theme.fg("dim", "←/↑ PgUp previous  →/↓ PgDn next  Home/End  Esc close")}`),
			this.theme.fg("border", `╰${"─".repeat(innerWidth)}╯`),
		];
	}

	invalidate(): void {}

	dispose(): void {
		if (this.isDisposed) return;
		this.isDisposed = true;
		clearInterval(this.timer);
	}

	private renderSpinnerPage(page: Extract<PreviewPage, { kind: "spinners" }>, row: (content: string) => string): string[] {
		const spinnerPage = Math.floor(page.startIndex / SPINNERS_PER_PAGE) + 1;
		const spinnerPageCount = Math.ceil(SPINNERS.length / SPINNERS_PER_PAGE);
		return [
			row(` ${this.theme.fg("accent", this.theme.bold("Spinner candidates"))}`),
			row(` ${this.theme.fg("muted", `Candidate set ${spinnerPage}/${spinnerPageCount}`)} ${this.theme.fg("dim", `page ${this.pageIndex + 1}/${PAGES.length}`)}`),
			row(""),
			...SPINNERS.slice(page.startIndex, page.startIndex + SPINNERS_PER_PAGE).map((spinner) => row(this.renderSpinner(spinner))),
		];
	}

	private renderInterfacePage(row: (content: string) => string): string[] {
		const now = Date.now();
		return [
			row(` ${this.theme.fg("accent", this.theme.bold("Interface motion"))}`),
			row(` ${this.theme.fg("muted", "Live loader and status behavior")} ${this.theme.fg("dim", `page ${this.pageIndex + 1}/${PAGES.length}`)}`),
			row(""),
			row(this.renderAdaptiveLoader(now)),
			row(this.renderStatusRail(now)),
			row(this.renderPairedMotion(now)),
			row(this.renderMotionBudget(now)),
			...this.renderLoadingIdentity(now).map(row),
		];
	}

	private renderPreInputSignalPage(row: (content: string) => string, innerWidth: number): string[] {
		const now = Date.now();
		const state = this.preInputState(now);
		return [
			row(` ${this.theme.fg("accent", this.theme.bold("Pre-input signal"))}`),
			row(` ${this.theme.fg("muted", "Live pre-input status behavior")} ${this.theme.fg("dim", `page ${this.pageIndex + 1}/${PAGES.length}`)}`),
			row(""),
			...this.renderPreInputSignal(state, innerWidth).map(row),
		];
	}

	private renderPreInputSignal(state: PreInputState, innerWidth: number): string[] {
		const project = this.theme.fg("muted", "agents");
		const branch = this.theme.fg("accent", "add-to-pi-config");
		const location = `${project}${this.theme.fg("dim", " > ")}${branch}`;
		const activityText = state.phase === "idle" ? "Ready" : this.formatElapsed(state.activeElapsedMilliseconds);
		const activity = this.theme.fg(state.phase === "idle" ? "success" : "dim", activityText);
		const leftWidth = visibleWidth(` agents > add-to-pi-config · ${activityText}`);
		const modelText = PRE_INPUT_MODEL_LABELS.find((label) => visibleWidth(label) <= Math.max(0, innerWidth - leftWidth)) ?? "";
		const railWidth = Math.max(0, innerWidth - leftWidth - visibleWidth(modelText));
		const rail = state.phase === "idle" ? " ".repeat(railWidth) : this.renderEncryptedStream(railWidth, state.phaseElapsedMilliseconds);
		const model = this.theme.fg("accent", modelText);
		return [` ${location}${this.theme.fg("dim", " · ")}${activity}${rail}${model}`];
	}

	private renderEncryptedStream(railWidth: number, elapsedMilliseconds: number): string {
		const streamTick = Math.floor(elapsedMilliseconds / 70);
		return Array.from({ length: railWidth }, (_, index) => {
			const seed = streamTick - (railWidth - 1 - index);
			if (seed < 0) return this.theme.fg("dim", "·");

			const age = streamTick - seed;
			const role = age < 4 ? "accent" : age < 12 ? "muted" : "dim";
			return this.theme.fg(role, this.encryptedCharacter(seed));
		}).join("");
	}

	private encryptedCharacter(seed: number): string {
		const value = Math.imul(seed + 1, 1_103_515_245) + 12_345;
		return ENCRYPTED_CHARACTERS[(value >>> 0) % ENCRYPTED_CHARACTERS.length]!;
	}

	private preInputState(now: number): PreInputState {
		const elapsedMilliseconds = (now - this.startedAt) % PRE_INPUT_CYCLE_MILLISECONDS;
		if (elapsedMilliseconds < PRE_INPUT_IDLE_MILLISECONDS) {
			return { phase: "idle", phaseElapsedMilliseconds: elapsedMilliseconds, activeElapsedMilliseconds: 0 };
		}
		const streamingElapsedMilliseconds = elapsedMilliseconds - PRE_INPUT_IDLE_MILLISECONDS;
		return {
			phase: "streaming",
			phaseElapsedMilliseconds: streamingElapsedMilliseconds,
			activeElapsedMilliseconds: streamingElapsedMilliseconds,
		};
	}

	private formatElapsed(milliseconds: number): string {
		const tenths = Math.floor(milliseconds / 100);
		const seconds = Math.floor(tenths / 10);
		return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}.${tenths % 10}`;
	}

	private renderSpinner(spinner: Spinner): string {
		const frameIndex = this.frameIndex(spinner.frames, spinner.intervalMs);
		const frame = spinner.name === "Arctic Aurora sweep"
			? this.renderArcticAurora(frameIndex)
			: this.theme.fg("accent", spinner.frames[frameIndex]!);
		return ` ${this.theme.fg("text", spinner.name.padEnd(21))}${frame} ${this.theme.fg("muted", `frame ${frameIndex + 1}/${spinner.frames.length}`)} ${this.theme.fg("dim", `interval ${spinner.intervalMs}ms`)}`;
	}

	private renderAdaptiveLoader(now: number): string {
		const elapsedMs = (now - this.startedAt) % 9_000;
		const elapsedSeconds = elapsedMs / 1_000;
		const state = elapsedSeconds < 3 ? "waiting" : elapsedSeconds < 6 ? "working" : "stalled";
		const indicator = state === "waiting"
			? this.theme.fg("muted", "···")
			: state === "working"
				? this.theme.fg("accent", this.frame(["◐", "◓", "◑", "◒"], 100, now))
				: this.theme.fg("warning", "▮");
		const stateRole = state === "waiting" ? "muted" : state === "working" ? "accent" : "warning";
		return ` ${this.theme.fg("text", "adaptive loader".padEnd(21))}${indicator} ${this.theme.fg(stateRole, state)} ${this.theme.fg("dim", `${elapsedSeconds.toFixed(1)}s`)}`;
	}

	private renderStatusRail(now: number): string {
		const railLength = 10;
		const markerIndex = Math.floor((now - this.startedAt) / 125) % railLength;
		const rail = Array.from({ length: railLength }, (_, index) =>
			index === markerIndex ? this.theme.fg("accent", "◆") : this.theme.fg("dim", "─"),
		).join("");
		return ` ${this.theme.fg("text", "status rail".padEnd(21))}${this.theme.fg("muted", "project > branch")} ${rail} ${this.theme.fg("muted", "model")}`;
	}

	private renderPairedMotion(now: number): string {
		const agent = this.theme.fg("accent", this.frame(["◐", "◓", "◑", "◒"], 100, now));
		const memory = this.theme.fg("dim", this.frame(["·  ", " · ", "  ·", " · "], 240, now));
		return ` ${this.theme.fg("text", "paired motion".padEnd(21))}${agent} ${this.theme.fg("accent", "agent")}  ${memory} ${this.theme.fg("muted", "memory")}`;
	}

	private renderMotionBudget(now: number): string {
		const fresh = this.theme.fg("accent", this.frame(["▁", "▃", "▆", "█", "▆", "▃"], 80, now));
		const stalled = this.theme.fg("warning", this.frame(["▁", "▃", "▆", "█", "▆", "▃"], 400, now));
		return ` ${this.theme.fg("text", "motion budget".padEnd(21))}${fresh} ${this.theme.fg("accent", "fresh 80ms")}  ${stalled} ${this.theme.fg("warning", "stalled 400ms")}`;
	}

	private renderLoadingIdentity(now: number): string[] {
		const working = this.theme.fg("accent", `${this.frame(["◐", "◓", "◑", "◒"], 100, now)} working`);
		const waiting = this.theme.fg("muted", "··· waiting");
		const retrying = this.theme.fg("warning", `${this.frame(["↻", "↺"], 220, now)} retrying`);
		const blocked = this.theme.fg("error", "× blocked");
		return [
			` ${this.theme.fg("text", "loading identity".padEnd(21))}${working}  ${waiting}`,
			` ${"".padEnd(21)}${retrying}  ${blocked}`,
		];
	}

	private frameIndex(frames: string[], intervalMs: number): number {
		return Math.floor((Date.now() - this.startedAt) / intervalMs) % frames.length;
	}

	private frame(frames: string[], intervalMs: number, now: number): string {
		return frames[Math.floor((now - this.startedAt) / intervalMs) % frames.length]!;
	}

	private renderArcticAurora(frameIndex: number): string {
		return Array.from({ length: 9 }, (_, index) => {
			const distance = Math.abs(index - frameIndex - 1);
			if (distance === 0) return this.theme.fg("accent", this.theme.bold("█"));
			if (distance === 1) return this.theme.fg("muted", "▓");
			if (distance === 2) return this.theme.fg("dim", "▒");
			return this.theme.fg("dim", "·");
		}).join("");
	}
}

export default function spinnerPreview(pi: ExtensionAPI): void {
	pi.registerCommand("spinner-preview", {
		description: "Compare animated loading and interface motion in the active theme.",
		handler: async (_args: string, ctx: ExtensionCommandContext) => {
		if (ctx.mode !== "tui") {
			ctx.ui.notify("Spinner preview requires interactive mode.", "error");
			return;
		}

		await ctx.ui.custom<void>(
			(tui, theme, _keybindings, done) => {
				let preview: SpinnerPreview;
				const close = () => {
					preview.dispose();
					done();
				};
				preview = new SpinnerPreview(theme, () => tui.requestRender(), close);
				return preview;
			},
			{
				overlay: true,
				overlayOptions: { anchor: "center", width: "80%", minWidth: 58, maxHeight: "90%", margin: 1 },
			},
		);
	},
	});
}
