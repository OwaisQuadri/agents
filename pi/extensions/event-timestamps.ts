import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

type TimestampEntry = {
	at: string;
	label: string;
};

const ENTRY_TYPE = "event-timestamp";

function isoTimestamp(value: unknown): string {
	const timestamp = typeof value === "number" && Number.isFinite(value) ? value : Date.now();
	return new Date(timestamp).toISOString();
}

function messageLabel(message: { role: string; toolName?: string }): string {
	if (message.role === "toolResult") return `tool result · ${message.toolName ?? "unknown"}`;
	return `${message.role} message`;
}

function displayTimestamp(value: string): string {
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return value;
	const parts = new Intl.DateTimeFormat("en-US", {
		weekday: "short",
		month: "short",
		day: "numeric",
		year: "numeric",
		hour: "numeric",
		minute: "2-digit",
		second: "2-digit",
		hour12: true,
	}).formatToParts(date);
	const part = (type: Intl.DateTimeFormatPartTypes) => parts.find((item) => item.type === type)?.value ?? "";
	return `${part("weekday")} ${part("month")} ${part("day")} ${part("year")} · ${part("hour")}:${part("minute")}:${part("second")} ${part("dayPeriod")}`;
}

export default function eventTimestamps(pi: ExtensionAPI): void {
	pi.registerEntryRenderer(ENTRY_TYPE, (entry, _options, theme) => {
		const data = entry.data as TimestampEntry;
		const line = `${displayTimestamp(data.at)} · ${data.label}`;
		return {
			render: (width) => [theme.fg("dim", line.slice(0, Math.max(0, width)))],
		};
	});

	pi.on("message_start", (event) => {
		pi.appendEntry(ENTRY_TYPE, {
			at: isoTimestamp(event.message.timestamp),
			label: messageLabel(event.message),
		});
	});

	pi.on("tool_call", (event) => {
		pi.appendEntry(ENTRY_TYPE, {
			at: isoTimestamp(Date.now()),
			label: `tool call · ${event.toolName}`,
		});
	});
}
