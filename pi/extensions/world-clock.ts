import { withFileMutationQueue, type ExtensionAPI, type ExtensionCommandContext } from "@earendil-works/pi-coding-agent";
import { visibleWidth } from "@earendil-works/pi-tui";
import { realpathSync } from "node:fs";
import { lstat, readFile, realpath, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

type ClockZone = {
	name: string;
	zone: string;
	color: string;
};

type WorldClockConfig = {
	is12Hour: boolean;
	zones: ClockZone[];
};

type ConfigureWorldClockInput = {
	action: "add" | "remove" | "reorder" | "recolor" | "reset";
	zone?: string;
	name?: string;
	color?: string;
	position?: number;
};

type WorldClockState = {
	render: (availableWidth: number) => string;
};

type ManagedConfig = {
	destinationPath: string;
	sourcePath: string;
};

type Alias = {
	name: string;
	zone: string;
};

const COLORS = ["#82b8ff", "#8ee7f5", "#a6dca8", "#c7b5ff", "#d394ff"];
const FORBIDDEN_COLORS = new Set(["#f4cf88", "#f28da5", "#f1c97a", "#f28b9a"]);
const ALIASES: Record<string, Alias> = {
	gmt: { name: "GMT", zone: "Etc/GMT" },
	london: { name: "London", zone: "Europe/London" },
	madrid: { name: "Madrid", zone: "Europe/Madrid" },
	"new-york": { name: "New York", zone: "America/New_York" },
	"los-angeles": { name: "Los Angeles", zone: "America/Los_Angeles" },
	"san-francisco": { name: "San Francisco", zone: "America/Los_Angeles" },
	chicago: { name: "Chicago", zone: "America/Chicago" },
	denver: { name: "Denver", zone: "America/Denver" },
	toronto: { name: "Toronto", zone: "America/Toronto" },
	seattle: { name: "Seattle", zone: "America/Los_Angeles" },
	dubai: { name: "Dubai", zone: "Asia/Dubai" },
	mecca: { name: "Mecca", zone: "Asia/Riyadh" },
	madinah: { name: "Madinah", zone: "Asia/Riyadh" },
	mumbai: { name: "Mumbai", zone: "Asia/Kolkata" },
	singapore: { name: "Singapore", zone: "Asia/Singapore" },
	tokyo: { name: "Tokyo", zone: "Asia/Tokyo" },
	sydney: { name: "Sydney", zone: "Australia/Sydney" },
};
const ALIAS_LIST = Object.keys(ALIASES).join(", ");
const CONFIGURATION_SCHEMA = {
	type: "object",
	additionalProperties: false,
	required: ["action"],
	properties: {
		action: { type: "string", enum: ["add", "remove", "reorder", "recolor", "reset"] },
		zone: { type: "string", description: `City alias (${ALIAS_LIST}) or IANA time zone for add` },
		name: { type: "string", description: "Configured segment name for remove, reorder, or recolor" },
		color: { type: "string", description: "Segment color as #RRGGBB" },
		position: { type: "integer", minimum: 0, description: "Zero-based position for reorder" },
	},
} as const;

function defaults(): WorldClockConfig {
	return {
		is12Hour: true,
		zones: [
			{ name: "Local", zone: "local", color: COLORS[0] },
			{ name: "GMT", zone: "Etc/GMT", color: COLORS[1] },
		],
	};
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isColor(value: string): boolean {
	return /^#[0-9a-fA-F]{6}$/.test(value) && !FORBIDDEN_COLORS.has(value.toLowerCase());
}

function isTimeZone(value: string): boolean {
	if (value === "local") return true;
	try {
		new Intl.DateTimeFormat("en-US", { timeZone: value }).format();
		return true;
	} catch {
		return false;
	}
}

function normalizeConfig(value: unknown): WorldClockConfig | undefined {
	if (!isRecord(value) || !Array.isArray(value.zones)) return undefined;
	const zones: ClockZone[] = [];
	for (const candidate of value.zones) {
		if (!isRecord(candidate)) return undefined;
		const { name, zone, color } = candidate;
		if (
			typeof name !== "string" || name.trim() === "" || typeof zone !== "string" || !isTimeZone(zone) ||
			typeof color !== "string" || !isColor(color)
		) {
			return undefined;
		}
		zones.push({ name: name.trim(), zone, color: color.toLowerCase() });
	}
	return { is12Hour: typeof value.is12Hour === "boolean" ? value.is12Hour : true, zones };
}

function systemZone(): string {
	return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

function resolvedZone(zone: string): string {
	return zone === "local" ? systemZone() : zone;
}

function formatZoneName(zone: string): string {
	return zone.split("/").at(-1)?.replaceAll("_", " ") ?? zone;
}

function offsetMinutes(zone: string, now: Date): number {
	const parts = new Intl.DateTimeFormat("en-US", {
		year: "numeric",
		month: "2-digit",
		day: "2-digit",
		hour: "2-digit",
		minute: "2-digit",
		second: "2-digit",
		hourCycle: "h23",
		timeZone: resolvedZone(zone),
	}).formatToParts(now);
	const value = (type: string) => Number(parts.find((part) => part.type === type)?.value ?? 0);
	return Math.round((Date.UTC(value("year"), value("month") - 1, value("day"), value("hour"), value("minute"), value("second")) - now.getTime()) / 60_000);
}

function createZone(input: string, color: string): ClockZone {
	const alias = ALIASES[input.toLowerCase()];
	if (alias) return { ...alias, color };
	if (!isTimeZone(input) || input === "local") {
		throw new Error(`Unknown city or IANA time zone: ${input}. Aliases: ${ALIAS_LIST}.`);
	}
	return { name: formatZoneName(input), zone: input, color };
}

function nextColor(config: WorldClockConfig): string {
	return COLORS[config.zones.length % COLORS.length];
}

function segmentIndex(config: WorldClockConfig, name: string): number {
	const target = name.trim().toLowerCase();
	return config.zones.findIndex((segment) => segment.name.toLowerCase() === target || segment.zone.toLowerCase() === target);
}

function stringifyConfig(config: WorldClockConfig): string {
	return `${JSON.stringify(config, null, 2)}\n`;
}

function configurationSummary(config: WorldClockConfig): string {
	const segments = config.zones.map((segment, index) => `${index}: ${segment.name} (${resolvedZone(segment.zone)}, ${segment.color})`);
	return `World clock: ${segments.length === 0 ? "no segments" : segments.join("; ")}. Aliases: ${ALIAS_LIST}.`;
}

function fgHex(hex: string | undefined, text: string, isBold: boolean): string {
	const weight = isBold ? "\x1b[1m" : "";
	const resetWeight = isBold ? "\x1b[22m" : "";
	if (hex === undefined) return `\x1b[39m${weight}${text}${resetWeight}\x1b[39m`;
	const [red, green, blue] = [1, 3, 5].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16));
	return `\x1b[38;2;${red};${green};${blue}m${weight}${text}${resetWeight}\x1b[39m`;
}

type StyledUnit = {
	text: string;
	color: string | undefined;
	isBold: boolean;
	width: number;
};

function units(text: string, color: string | undefined, isBold: boolean): StyledUnit[] {
	return Array.from(text).map((character) => ({ text: character, color, isBold, width: visibleWidth(character) }));
}

function renderUnits(unitsToRender: StyledUnit[]): string {
	let rendered = "";
	let text = "";
	let color: string | undefined;
	let isBold = false;
	for (const unit of unitsToRender) {
		if (text !== "" && (unit.color !== color || unit.isBold !== isBold)) {
			rendered += fgHex(color, text, isBold);
			text = "";
		}
		text += unit.text;
		color = unit.color;
		isBold = unit.isBold;
	}
	return text === "" ? rendered : rendered + fgHex(color, text, isBold);
}

export function renderClock(config: WorldClockConfig, availableWidth: number): string {
	if (availableWidth <= 0 || config.zones.length === 0) return "";
	const now = new Date();
	const sortedZones = config.zones
		.map((segment, index) => ({ segment, index }))
		.sort((left, right) => {
			const offset = offsetMinutes(left.segment.zone, now) - offsetMinutes(right.segment.zone, now);
			return offset === 0 ? left.index - right.index : offset;
		})
		.map(({ segment }) => segment);
	const groupedSegments = new Map<string, ClockZone[]>();
	for (const segment of sortedZones) {
		const time = new Intl.DateTimeFormat("en-US", {
			hour: "numeric",
			minute: "2-digit",
			hour12: config.is12Hour,
			timeZone: resolvedZone(segment.zone),
		}).format();
		const group = groupedSegments.get(time) ?? [];
		group.push(segment);
		groupedSegments.set(time, group);
	}
	const separator = units(" · ", "#7f8caa", false);
	const content = Array.from(groupedSegments.entries()).flatMap(([time, segments], groupIndex) => [
		...(groupIndex === 0 ? [] : separator),
		...segments.flatMap((segment, segmentIndex) => [
			...(segmentIndex === 0 ? [] : units("/", "#7f8caa", false)),
			...units(segment.name, segment.color, segment.zone === "local"),
		]),
		...units(` ${time}`, segments.some((segment) => segment.zone === "local") ? undefined : "#7d8eae", segments.some((segment) => segment.zone === "local")),
	]);
	const contentWidth = content.reduce((sum, unit) => sum + unit.width, 0);
	if (contentWidth <= availableWidth) return renderUnits(content);
	const loop = [...content, ...separator];
	const loopWidth = loop.reduce((sum, unit) => sum + unit.width, 0);
	const offset = Math.floor(Date.now() / 500) % loopWidth;
	let consumed = 0;
	let index = 0;
	while (consumed + loop[index].width <= offset) {
		consumed += loop[index].width;
		index = (index + 1) % loop.length;
	}
	const window: StyledUnit[] = [];
	let width = 0;
	while (width < availableWidth) {
		const unit = loop[index];
		if (unit.width > 0 && width + unit.width > availableWidth) break;
		window.push(unit);
		width += unit.width;
		index = (index + 1) % loop.length;
	}
	return renderUnits(window);
}

function managedDestinationPath(): string {
	return join(homedir(), ".pi", "agent", "world-clock.json");
}

function sourcePath(repositoryRoot: string): string {
	return resolve(repositoryRoot, "config", "world-clock.json");
}

async function managedConfig(repositoryRoot: string): Promise<ManagedConfig> {
	const destinationPath = managedDestinationPath();
	let metadata;
	try {
		metadata = await lstat(destinationPath);
	} catch {
		throw new Error(`Managed world-clock configuration is missing: ${destinationPath}`);
	}
	if (!metadata.isSymbolicLink()) {
		throw new Error(`Refusing to change ${destinationPath}: it is not a managed symlink.`);
	}
	const expectedSource = await realpath(sourcePath(repositoryRoot));
	const resolvedDestination = await realpath(destinationPath);
	if (resolvedDestination !== expectedSource) {
		throw new Error(`Refusing to change ${destinationPath}: it does not resolve to ${expectedSource}.`);
	}
	return { destinationPath, sourcePath: expectedSource };
}

async function readManagedConfig(repositoryRoot: string): Promise<{ config: WorldClockConfig; managed: ManagedConfig }> {
	const managed = await managedConfig(repositoryRoot);
	const config = normalizeConfig(JSON.parse(await readFile(managed.destinationPath, "utf8")));
	if (!config) throw new Error(`World-clock configuration at ${managed.destinationPath} is invalid.`);
	return { config, managed };
}

async function loadDisplayConfig(repositoryRoot: string): Promise<WorldClockConfig> {
	try {
		return (await readManagedConfig(repositoryRoot)).config;
	} catch {
		return defaults();
	}
}

async function mutateConfig(
	repositoryRoot: string,
	mutate: (config: WorldClockConfig) => WorldClockConfig,
): Promise<WorldClockConfig> {
	const first = await managedConfig(repositoryRoot);
	return withFileMutationQueue(first.sourcePath, async () => {
		const { config, managed } = await readManagedConfig(repositoryRoot);
		const updated = mutate(config);
		await writeFile(managed.sourcePath, stringifyConfig(updated));
		return updated;
	});
}

function addZone(config: WorldClockConfig, zoneInput: string, colorInput: string | undefined): WorldClockConfig {
	const color = colorInput ?? nextColor(config);
	if (!isColor(color)) throw new Error("Color must be a #RRGGBB value.");
	const zone = createZone(zoneInput, color.toLowerCase());
	const isDuplicate = config.zones.some((segment) => resolvedZone(segment.zone) === resolvedZone(zone.zone));
	if (isDuplicate) throw new Error(`${zone.name} is already configured.`);
	return { ...config, zones: [...config.zones, zone] };
}

function removeZone(config: WorldClockConfig, name: string): WorldClockConfig {
	const index = segmentIndex(config, name);
	if (index < 0) throw new Error(`No world-clock segment named ${name}.`);
	return { ...config, zones: config.zones.filter((_, candidate) => candidate !== index) };
}

function reorderZone(config: WorldClockConfig, name: string, position: number): WorldClockConfig {
	const index = segmentIndex(config, name);
	if (index < 0) throw new Error(`No world-clock segment named ${name}.`);
	if (!Number.isInteger(position) || position < 0 || position >= config.zones.length) {
		throw new Error(`Position must be from 0 to ${Math.max(config.zones.length - 1, 0)}.`);
	}
	const zones = [...config.zones];
	const [segment] = zones.splice(index, 1);
	zones.splice(position, 0, segment);
	return { ...config, zones };
}

function recolorZone(config: WorldClockConfig, name: string, color: string): WorldClockConfig {
	const index = segmentIndex(config, name);
	if (index < 0) throw new Error(`No world-clock segment named ${name}.`);
	if (!isColor(color)) throw new Error("Color must be a #RRGGBB value.");
	return {
		...config,
		zones: config.zones.map((segment, candidate) => candidate === index ? { ...segment, color: color.toLowerCase() } : segment),
	};
}

function requireValue(value: string | undefined, description: string): string {
	if (!value || value.trim() === "") throw new Error(`${description} is required.`);
	return value.trim();
}

function commandFailure(ctx: ExtensionCommandContext, error: unknown): void {
	ctx.ui.notify(error instanceof Error ? error.message : String(error), "error");
}

export default function worldClock(pi: ExtensionAPI): void {
	const extensionPath = realpathSync(fileURLToPath(import.meta.url));
	const repositoryRoot = resolve(dirname(extensionPath), "../..");
	let config = defaults();
	const state: WorldClockState = { render: (availableWidth) => renderClock(config, availableWidth) };
	(globalThis as typeof globalThis & { __owaisWorldClockState?: WorldClockState }).__owaisWorldClockState = state;

	async function refreshConfig(): Promise<void> {
		config = await loadDisplayConfig(repositoryRoot);
	}

	async function update(mutator: (current: WorldClockConfig) => WorldClockConfig): Promise<WorldClockConfig> {
		config = await mutateConfig(repositoryRoot, mutator);
		return config;
	}

	pi.on("session_start", async () => {
		await refreshConfig();
	});

	pi.on("session_shutdown", () => {
		const globals = globalThis as typeof globalThis & { __owaisWorldClockState?: WorldClockState };
		if (globals.__owaisWorldClockState === state) delete globals.__owaisWorldClockState;
	});

	pi.registerCommand("tz", {
		description: "Manage world-clock segments: add, remove, list, or reset",
		handler: async (args, ctx) => {
			const [action, ...rest] = args.trim().split(/\s+/);
			try {
				switch (action) {
					case "list":
						if (rest.length > 0) throw new Error("Usage: /tz list");
						config = (await readManagedConfig(repositoryRoot)).config;
						ctx.ui.notify(configurationSummary(config), "info");
						return;
					case "add": {
						if (rest.length < 1 || rest.length > 2) throw new Error("Usage: /tz add <city-or-IANA-zone> [#RRGGBB]");
						const updated = await update((current) => addZone(current, rest[0], rest[1]));
						ctx.ui.notify(configurationSummary(updated), "info");
						return;
					}
					case "remove": {
						const name = rest.join(" ").trim();
						if (!name) throw new Error("Usage: /tz remove <name>");
						const updated = await update((current) => removeZone(current, name));
						ctx.ui.notify(configurationSummary(updated), "info");
						return;
					}
					case "reset":
						if (rest.length > 0) throw new Error("Usage: /tz reset");
						const updated = await update(() => defaults());
						ctx.ui.notify(configurationSummary(updated), "info");
						return;
					default:
						throw new Error("Usage: /tz add <city-or-IANA-zone> [#RRGGBB], /tz remove <name>, /tz list, or /tz reset");
				}
			} catch (error) {
				commandFailure(ctx, error);
			}
		},
	});

	pi.registerTool({
		name: "configure_world_clock",
		label: "Configure World Clock",
		description: "Add, remove, reorder, recolor, or reset managed world-clock segments. Add accepts exact IANA zones and these aliases: " + ALIAS_LIST + ".",
		promptSnippet: "Configure managed world-clock segments",
		promptGuidelines: ["Use configure_world_clock when the user asks to change world-clock segments."],
		parameters: CONFIGURATION_SCHEMA,
		async execute(_toolCallId, params: ConfigureWorldClockInput) {
			let updated = config;
			switch (params.action) {
				case "add":
					updated = await update((current) => addZone(current, requireValue(params.zone, "Zone"), params.color));
					break;
				case "remove":
					updated = await update((current) => removeZone(current, requireValue(params.name, "Name")));
					break;
				case "reorder":
					updated = await update((current) => reorderZone(current, requireValue(params.name, "Name"), params.position ?? -1));
					break;
				case "recolor":
					updated = await update((current) => recolorZone(current, requireValue(params.name, "Name"), requireValue(params.color, "Color")));
					break;
				case "reset":
					updated = await update(() => defaults());
			}
			return {
				content: [{ type: "text", text: configurationSummary(updated) }],
				details: { config: updated },
			};
		},
	});
}
