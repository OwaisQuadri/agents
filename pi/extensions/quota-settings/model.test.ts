import assert from "node:assert/strict";
import { test } from "node:test";

import { parseQuotaOverride, updateQuotaOverride } from "./model.ts";

test("parseQuotaOverride accepts each provider", () => {
	assert.equal(parseQuotaOverride('{"quotaAdmission":{"overrideProvider":"anthropic"}}'), "anthropic");
	assert.equal(parseQuotaOverride('{"quotaAdmission":{"overrideProvider":"openai-codex"}}'), "openai-codex");
});

test("parseQuotaOverride treats null and unknown values as no override", () => {
	assert.equal(parseQuotaOverride('{"quotaAdmission":{"overrideProvider":null}}'), null);
	assert.equal(parseQuotaOverride('{"quotaAdmission":{"overrideProvider":"other"}}'), null);
	assert.equal(parseQuotaOverride("{}"), null);
});

test("parseQuotaOverride rejects malformed JSON", () => {
	assert.throws(() => parseQuotaOverride("{ not json"));
});

test("updateQuotaOverride preserves unrelated settings", () => {
	const updated = JSON.parse(updateQuotaOverride('{"theme":"owais","quotaAdmission":{"other":true}}', "anthropic"));
	assert.deepEqual(updated, {
		theme: "owais",
		quotaAdmission: { other: true, overrideProvider: "anthropic" },
	});
});

test("updateQuotaOverride writes null to clear an installed override", () => {
	const updated = JSON.parse(updateQuotaOverride('{"quotaAdmission":{"overrideProvider":"openai-codex"}}', null));
	assert.deepEqual(updated, { quotaAdmission: { overrideProvider: null } });
});

test("updateQuotaOverride rejects a non-object settings document", () => {
	assert.throws(() => updateQuotaOverride("[]", "anthropic"), /top-level object/);
});
