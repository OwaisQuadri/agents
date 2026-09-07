import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { buildKickoffPrompt } from "./prompt.ts";

const cases = JSON.parse(
  readFileSync(new URL("./architecture-cases.json", import.meta.url), "utf8"),
);
const whitelistDocText = readFileSync(
  new URL("../../../../docs/comment-style.md", import.meta.url),
  "utf8",
);
const requiredFields = [
  "id",
  "commentText",
  "followingContext",
  "expectedShape",
  "rationale",
];

test("architecture fixtures have four unique cases and complete fields", () => {
  assert.ok(Array.isArray(cases));
  assert.equal(cases.length, 4);
  const ids = new Set<string>();
  for (const fixture of cases) {
    assert.ok(fixture !== null && typeof fixture === "object");
    assert.deepEqual(Object.keys(fixture).sort(), [...requiredFields].sort());
    for (const field of requiredFields) {
      assert.equal(typeof fixture[field], "string", `${fixture.id}: ${field}`);
      assert.ok(fixture[field].trim().length > 0, `${fixture.id}: ${field}`);
    }
    assert.ok(!ids.has(fixture.id), `duplicate fixture: ${fixture.id}`);
    ids.add(fixture.id);
    const lines = fixture.commentText.split(/\r\n|\r|\n/);
    assert.ok(lines.length <= 3, `${fixture.id}: comment exceeds three lines`);
    assert.ok(lines.every((line: string) => /^\/\/ (?!\/).+/.test(line)));
  }
});

test("architecture fixtures contain two blocked and two allowed expectations", () => {
  assert.deepEqual(
    cases.map((fixture) => fixture.expectedShape).sort(),
    [
      "inexpressible concept or architecture",
      "inexpressible concept or architecture",
      "none",
      "none",
    ],
  );
});

test("kickoff prompts preserve fixture text and the supplied whitelist verbatim", () => {
  for (const fixture of cases) {
    const prompt = buildKickoffPrompt({
      commentText: fixture.commentText,
      followingContext: fixture.followingContext,
      whitelistDocText,
      language: "typescript",
    });
    assert.ok(prompt.includes(fixture.commentText), `${fixture.id}: comment`);
    assert.ok(prompt.includes(fixture.followingContext), `${fixture.id}: context`);
    assert.ok(prompt.includes(whitelistDocText), `${fixture.id}: whitelist`);
    assert.ok(!prompt.includes(fixture.rationale), `${fixture.id}: rationale leaked`);
  }
});
