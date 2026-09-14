import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const PLAN_SUBMIT_TOOL = "plannotator_submit_plan";
const REQUEST_CHANNEL = "plannotator:request";
const REQUEST_TIMEOUT_MS = 5_000;
const MAX_ERROR_LENGTH = 160;

type PlanModeResponse = {
	status?: unknown;
	result?: { phase?: unknown };
	error?: unknown;
};

type ActivationDetails = {
	isSuccess: boolean;
	status: "planning" | "unavailable" | "error" | "malformed" | "wrong-phase" | "missing-tool" | "timeout" | "cancelled";
	error?: string;
};

type ActivationResult = {
	content: [{ type: "text"; text: string }];
	details: ActivationDetails;
};

function boundedError(error: unknown): string | undefined {
	if (typeof error !== "string") return undefined;
	return error.slice(0, MAX_ERROR_LENGTH);
}

function result(status: ActivationDetails["status"], error?: unknown): ActivationResult {
	const isSuccess = status === "planning";
	const details: ActivationDetails = { isSuccess, status };
	const bounded = boundedError(error);
	if (bounded !== undefined) details.error = bounded;
	return {
		content: [{ type: "text", text: isSuccess ? "Plannotator plan mode is active." : `Plannotator plan mode ${status}.` }],
		details,
	};
}

function isPlanModeResponse(value: unknown): value is PlanModeResponse {
	return value !== null && typeof value === "object";
}

export async function enterPlanMode(
	pi: Pick<ExtensionAPI, "getActiveTools" | "events">,
	toolCallId: string,
	signal: AbortSignal | undefined,
	timeoutMs = REQUEST_TIMEOUT_MS,
): Promise<ActivationResult> {
	if (pi.getActiveTools().includes(PLAN_SUBMIT_TOOL)) return result("planning");
	if (signal?.aborted) return result("cancelled");

	return new Promise((resolve) => {
		let isSettled = false;
		const settle = (value: ActivationResult): void => {
			if (isSettled) return;
			isSettled = true;
			clearTimeout(timeout);
			signal?.removeEventListener("abort", onAbort);
			resolve(value);
		};
		const onAbort = (): void => settle(result("cancelled"));
		const timeout = setTimeout(() => settle(result("timeout")), timeoutMs);
		const respond = (response: unknown): void => {
			if (!isPlanModeResponse(response)) {
				settle(result("malformed"));
				return;
			}
			if (response.status === "unavailable" || response.status === "error") {
				settle(result(response.status, response.error));
				return;
			}
			if (response.status !== "handled" || !isPlanModeResponse(response.result)) {
				settle(result("malformed"));
				return;
			}
			if (response.result.phase !== "planning") {
				settle(result("wrong-phase"));
				return;
			}
			if (!pi.getActiveTools().includes(PLAN_SUBMIT_TOOL)) {
				settle(result("missing-tool"));
				return;
			}
			settle(result("planning"));
		};

		signal?.addEventListener("abort", onAbort, { once: true });
		try {
			pi.events.emit(REQUEST_CHANNEL, {
				requestId: toolCallId,
				action: "plan-mode",
				payload: { mode: "enter" },
				respond,
			});
		} catch (error) {
			settle(result("unavailable", error instanceof Error ? error.message : undefined));
		}
	});
}
