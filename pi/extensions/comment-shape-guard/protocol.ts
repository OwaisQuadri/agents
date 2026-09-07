export type JudgmentInput = {
	comment: string;
	code_context: string;
	language: string;
	rule_document: string;
	prompt_version: number;
	schema_version: number;
	judgment_configuration: string;
};
export type JudgmentRequest = { line: number; input: JudgmentInput };
export type Request = {
	version: 1;
	request_id: string;
	operation: "edit" | "write";
	path: string;
	repository_root: string | null;
	original_text: string | null;
	proposed_text: string;
	changed_ranges: { start_byte: number; end_byte: number }[];
	budget_ms: number;
};
export type Diagnostic = { path: string; line: number; rule: "privacy" | "comment-length" | "comment-shape" | "boolean-name" | "checker"; reason: string };
export type Response = {
	version: 1;
	request_id: string;
	decision: "pass" | "block" | "needs_judgment" | "error";
	diagnostics: Diagnostic[];
	judgments: JudgmentRequest[];
	elapsed_ms: number;
	budget_ms: number;
};

function object(value: unknown, keys: string[]): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).length !== keys.length || !keys.every((key) => Object.hasOwn(value, key))) throw new Error("Invalid checker response; repair the required checker.");
	return value as Record<string, unknown>;
}

function isInteger(value: unknown, minimum: number): value is number {
	return Number.isSafeInteger(value) && (value as number) >= minimum;
}

export function validateJudgmentInput(value: unknown): JudgmentInput {
	const input = object(value, ["comment", "code_context", "language", "rule_document", "prompt_version", "schema_version", "judgment_configuration"]);
	for (const field of ["comment", "code_context", "language", "rule_document", "judgment_configuration"]) {
		if (typeof input[field] !== "string" || !(input[field] as string).isWellFormed()) throw new Error("Invalid judgment input.");
	}
	if (input.prompt_version !== 1 || input.schema_version !== 1 || !input.rule_document || !input.judgment_configuration || !input.language) throw new Error("Unsupported judgment configuration or version.");
	return input as JudgmentInput;
}

export function validateResponse(raw: string, request: Request): Response {
	const result = object(JSON.parse(raw), ["version", "request_id", "decision", "diagnostics", "judgments", "elapsed_ms", "budget_ms"]);
	if (!isInteger(result.budget_ms, 1) || result.budget_ms > request.budget_ms || result.budget_ms > 20_000) throw new Error("Invalid checker budget.");
	if (result.version !== 1 || result.request_id !== request.request_id || !isInteger(result.elapsed_ms, 0) || result.elapsed_ms > request.budget_ms || !["pass", "block", "needs_judgment", "error"].includes(result.decision as string) || !Array.isArray(result.diagnostics) || !Array.isArray(result.judgments)) throw new Error("Invalid checker response.");
	const maxLine = request.proposed_text.split("\n").length;
	for (const value of result.diagnostics) {
		const diagnostic = object(value, ["path", "line", "rule", "reason"]);
		if (diagnostic.path !== request.path || !isInteger(diagnostic.line, 1) || diagnostic.line > maxLine || !["privacy", "comment-length", "comment-shape", "boolean-name", "checker"].includes(diagnostic.rule as string) || diagnostic.rule === "checker" && result.decision !== "error" || typeof diagnostic.reason !== "string" || !diagnostic.reason) throw new Error("Invalid checker diagnostic.");
	}
	for (const value of result.judgments) {
		const judgment = object(value, ["line", "input"]);
		if (!isInteger(judgment.line, 1) || judgment.line > maxLine) throw new Error("Invalid judgment source position.");
		validateJudgmentInput(judgment.input);
	}
	if (result.decision === "pass" && (result.diagnostics.length || result.judgments.length) || result.decision === "needs_judgment" && (!result.judgments.length || result.diagnostics.length) || ["block", "error"].includes(result.decision as string) && (!result.diagnostics.length || result.judgments.length)) throw new Error("Invalid checker decision combination.");
	return result as Response;
}

const guidance: Record<Diagnostic["rule"], string> = {
	privacy: "Remove or replace the private value with a placeholder.",
	"comment-length": "Shorten the non-documentation comment to at most three lines.",
	"comment-shape": "Remove the comment or use an approved comment shape.",
	"boolean-name": "Rename the proven Boolean declaration with an is prefix.",
	checker: "Repair the required checker, rules, configuration, or worker and retry.",
};

export function blockingMessage(diagnostics: Pick<Diagnostic, "path" | "line" | "rule">[], elapsed: number): string {
	const header = `Blocked before write (${Math.ceil(elapsed)}ms).`;
	const lines: string[] = [];
	const footer = (omitted: number) => `\n${omitted} diagnostics omitted.`;
	let length = header.length;
	for (const diagnostic of diagnostics) {
		if (diagnostic.path.length > 2000) break;
		const line = `\n${JSON.stringify(diagnostic.path)}:${diagnostic.line}: ${diagnostic.rule}: ${guidance[diagnostic.rule]}`;
		if (length + line.length + footer(diagnostics.length - lines.length - 1).length > 2000) break;
		lines.push(line);
		length += line.length;
	}
	return `${header}${lines.join("")}${footer(diagnostics.length - lines.length)}`;
}
