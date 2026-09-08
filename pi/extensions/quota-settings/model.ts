export type QuotaProvider = "anthropic" | "openai-codex";

export const QUOTA_PROVIDERS: QuotaProvider[] = ["anthropic", "openai-codex"];

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isQuotaProvider(value: unknown): value is QuotaProvider {
	return typeof value === "string" && QUOTA_PROVIDERS.includes(value as QuotaProvider);
}

export function parseQuotaOverride(raw: string): QuotaProvider | null {
	const parsed: unknown = JSON.parse(raw);
	if (!isRecord(parsed)) return null;
	const quotaAdmission = parsed.quotaAdmission;
	if (!isRecord(quotaAdmission)) return null;
	return isQuotaProvider(quotaAdmission.overrideProvider) ? quotaAdmission.overrideProvider : null;
}

export function updateQuotaOverride(raw: string, provider: QuotaProvider | null): string {
	const parsed: unknown = JSON.parse(raw);
	if (!isRecord(parsed)) throw new Error("Pi settings must be a top-level object");
	const current = isRecord(parsed.quotaAdmission) ? parsed.quotaAdmission : {};
	return `${JSON.stringify(
		{
			...parsed,
			quotaAdmission: { ...current, overrideProvider: provider },
		},
		null,
		2,
	)}\n`;
}
