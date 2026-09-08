import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("./quota-settings.ts", import.meta.url), "utf8");

test("/quota registers a separate command", () => {
	assert.match(source, /registerCommand\("quota"/);
});

test("/quota uses the same in-place settings list pattern as /tiers", () => {
	assert.match(source, /new SettingsList\(/);
	assert.match(source, /getSettingsListTheme\(\)/);
	assert.match(source, /ctx\.ui\.custom<undefined>/);
	assert.doesNotMatch(source, /overlay:\s*true/);
});

test("/quota presents the provider and explicit apply rows", () => {
	assert.match(source, /label: "Provider override"/);
	assert.match(source, /"No override", "Anthropic", "OpenAI Codex"/);
	assert.match(source, /label: "Apply changes"/);
});

test("/quota rereads managed settings before writing and running install.sh", () => {
	const readIndex = source.indexOf('const currentSettings = await readFile(path, "utf8")');
	const writeIndex = source.indexOf("await writeFile(path, updateQuotaOverride(currentSettings");
	const installIndex = source.indexOf("result = await runInstall(repoRoot)");
	assert.notEqual(readIndex, -1);
	assert.notEqual(writeIndex, -1);
	assert.notEqual(installIndex, -1);
	assert.ok(readIndex < writeIndex && writeIndex < installIndex);
});
